//! A run's way out of its microVM, and the record of what went through it.
//!
//! A run's guest has no network device. When the runner gives a run a way
//! out, it opens an [`Egress`] in the run's directory, on the socket name
//! the launcher connects to (Firecracker's `v.sock_3128` for vsock port
//! 3128), and hands on what it gives: [`Egress::port`] to the launcher,
//! which joins the guest's `127.0.0.1:3128` to that socket, and
//! [`Egress::guest_env`] to the command, whose tools then send
//! `CONNECT host:port` to that address. [`Egress::close`] ends it and
//! returns a [`Summary`]. The runner touches nothing else of this crate.
//!
//! **Behind an egress runs `fence proxy`**, one process per run: fence's
//! proxy without a box, which lets through only the hosts the [`Policy`]
//! names, checks that a name does not lead to a loopback, private or
//! link-local address, and relays what it lets through unread, so TLS stays
//! end to end. [`Egress::open`] starts it in the run's directory and returns
//! once it says it listens: that is the signal to start the VM.
//! [`Egress::close`] closes its standard input, which ends it, and counts
//! its ledger. If kai's process ends first, the closed pipe ends fence too;
//! dropping an [`Egress`] kills it.
//!
//! **What kai relies on**, and the tests hold a stand-in program to:
//!
//! - `fence proxy --listen=unix:<socket> [--net=<host:port>]…
//!   --ledger=proxy.jsonl [--learn]`, started in the run's directory, so
//!   the socket's path is relative and its length does not matter;
//! - one line on standard output, `listening unix:<socket>`, once the
//!   socket listens, and nothing else there ever;
//! - exit status 0 once its standard input closes; 2 for a command line it
//!   refuses and 1 when it cannot listen or open its ledger, both before
//!   the line;
//! - its ledger, `proxy.jsonl` in the run's directory: one JSON line per
//!   decision, of which kai reads those with `"kind": "net"`: `host`,
//!   `port` and `decision` (`allow`, `deny`, or `would-deny` in learn
//!   mode).
//!
//! fence gets `PATH` and `HOME` from kai's environment and nothing else, so
//! no token of kai's reaches it.
//!
//! ```no_run
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use proxy::{Egress, Policy};
//!
//! let run_dir = std::path::Path::new("runs/r1");
//! let policy = Policy::new(["index.crates.io:443", "static.crates.io:443"]);
//! let egress = Egress::open(run_dir, "v.sock_3128", &policy).await?;
//! let (port, socket) = egress.port();
//! assert_eq!(port, 3128);
//! assert_eq!(socket, run_dir.join("v.sock_3128"));
//! // … start the VM with that port and egress.guest_env(), wait for it …
//! let summary = egress.close().await?;
//! println!("{} allowed, refused: {:?}", summary.allowed, summary.refused_targets);
//! # Ok(())
//! # }
//! ```

mod fence;
mod ledger;
mod policy;

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fence::Fence;
use ledger::LEDGER;

pub use ledger::Summary;
pub use policy::Policy;

/// The port the guest's tools are pointed at, on the guest's `127.0.0.1`.
pub const GUEST_PORT: u16 = 3128;

/// Why an egress could not be opened or closed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The socket's name is not a plain file name of its own.
    #[error(
        "{name:?} is no name for the socket: a file name in the run's directory, not {LEDGER:?}"
    )]
    BadName {
        /// The name given.
        name: String,
    },
    /// A file is where the egress would make its socket or its ledger.
    #[error("{} exists already; an egress never takes over a file it did not make", .path.display())]
    Exists {
        /// The file in the way.
        path: PathBuf,
    },
    /// The fence program was not found.
    #[error("{} was not found; `kai vm doctor` says what is missing", .program.display())]
    FenceMissing {
        /// The program as the policy names it.
        program: PathBuf,
    },
    /// fence ended before it listened: a command line or policy it
    /// refused, or a socket or ledger it could not make.
    #[error("fence proxy ended before it listened ({how}): {stderr}")]
    Refused {
        /// How it ended.
        how: String,
        /// What it said on standard error.
        stderr: String,
    },
    /// fence did not say it listens in time, and was killed.
    #[error("fence proxy did not listen within {after:?}, and was killed: {stderr}")]
    NotReady {
        /// The time it had.
        after: Duration,
        /// What it said on standard error.
        stderr: String,
    },
    /// fence's first line was not the one that says it listens; it was
    /// killed.
    #[error("fence proxy wrote {line:?} where it says it listens, and was killed")]
    UnexpectedLine {
        /// The line, cut to 200 characters.
        line: String,
    },
    /// fence did not end cleanly when the egress was closed.
    #[error("fence proxy did not end cleanly ({how}); its ledger may be incomplete: {stderr}")]
    Ended {
        /// How it ended, or that it was killed.
        how: String,
        /// What it said on standard error.
        stderr: String,
    },
    /// A ledger line kai cannot read.
    #[error("{}, line {line}: {reason}", .path.display())]
    Ledger {
        /// The ledger.
        path: PathBuf,
        /// The line's number, from 1.
        line: usize,
        /// What is wrong with it.
        reason: String,
    },
    /// The file system or the process refused.
    #[error("{what} {}: {source}", .path.display())]
    Io {
        /// What the egress was doing.
        what: &'static str,
        /// The file or program it was doing it to.
        path: PathBuf,
        /// What the system said.
        #[source]
        source: io::Error,
    },
    /// A task of the egress' own ended abnormally.
    #[error("a task of the egress failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

/// A run's way out: `fence proxy` on a socket in the run's directory, and
/// its ledger beside it.
///
/// Dropping an egress kills fence without waiting: close it, so that the
/// ledger is complete and counted.
#[derive(Debug)]
pub struct Egress {
    socket: PathBuf,
    ledger: PathBuf,
    fence: Fence,
    close_grace: Duration,
}

impl Egress {
    /// Starts `fence proxy` in `dir`, the run's directory, which must
    /// exist, listening on a socket of the name `socket` there: the name
    /// the launcher connects to. Returns once fence says it listens.
    /// Refuses to take over a file of that name or a `proxy.jsonl` that is
    /// there already, and then starts nothing.
    ///
    /// Needs a tokio runtime with I/O and timers enabled; the program that
    /// calls it runs one.
    pub async fn open(dir: &Path, socket: &str, policy: &Policy) -> Result<Egress, Error> {
        let (socket_path, ledger) = check(dir, socket).await?;
        let fence = Fence::start(dir, socket, policy).await?;
        tracing::debug!(socket = %socket_path.display(), rules = policy.net.len(), "egress open");
        Ok(Egress {
            socket: socket_path,
            ledger,
            fence,
            close_grace: policy.close_grace,
        })
    }

    /// For the launcher: the port the guest's tools reach on its
    /// `127.0.0.1`, and the host socket it ends at.
    pub fn port(&self) -> (u16, &Path) {
        (GUEST_PORT, &self.socket)
    }

    /// For the guest's command: the proxy variables, in upper and lower
    /// case, pointing at [`GUEST_PORT`].
    pub fn guest_env(&self) -> Vec<(&'static str, String)> {
        let url = format!("http://127.0.0.1:{GUEST_PORT}");
        let none = "localhost,127.0.0.1,::1".to_owned();
        vec![
            ("HTTPS_PROXY", url.clone()),
            ("HTTP_PROXY", url.clone()),
            ("ALL_PROXY", url.clone()),
            ("https_proxy", url.clone()),
            ("http_proxy", url.clone()),
            ("all_proxy", url),
            ("NO_PROXY", none.clone()),
            ("no_proxy", none),
        ]
    }

    /// The ledger: `proxy.jsonl` in the run's directory, fence's.
    pub fn ledger(&self) -> &Path {
        &self.ledger
    }

    /// Closes fence's standard input, which ends it, waits for it, and
    /// counts its ledger. A fence that has not ended within the policy's
    /// grace is killed, and that is an error: its ledger may lack lines.
    pub async fn close(self) -> Result<Summary, Error> {
        self.fence.end(self.close_grace).await?;
        let text = tokio::fs::read_to_string(&self.ledger)
            .await
            .map_err(|source| Error::Io {
                what: "cannot read the ledger",
                path: self.ledger.clone(),
                source,
            })?;
        ledger::summarize(&text).map_err(|(line, reason)| Error::Ledger {
            path: self.ledger.clone(),
            line,
            reason,
        })
    }
}

/// What `open` checks before it starts fence: a socket name that is a
/// plain file name of its own, a run directory that is there, and no file
/// where the socket or the ledger would go. Returns their paths.
async fn check(dir: &Path, socket: &str) -> Result<(PathBuf, PathBuf), Error> {
    let plain = !socket.is_empty()
        && socket != "."
        && socket != ".."
        && socket != LEDGER
        && !socket.contains(['/', '\0']);
    if !plain {
        return Err(Error::BadName {
            name: socket.to_owned(),
        });
    }
    match tokio::fs::metadata(dir).await {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            return Err(Error::Io {
                what: "not a directory:",
                path: dir.to_owned(),
                source: io::Error::from(io::ErrorKind::NotADirectory),
            });
        }
        Err(source) => {
            return Err(Error::Io {
                what: "cannot use the run directory",
                path: dir.to_owned(),
                source,
            });
        }
    }
    let socket_path = dir.join(socket);
    let ledger = dir.join(LEDGER);
    for path in [&socket_path, &ledger] {
        if tokio::fs::symlink_metadata(path).await.is_ok() {
            return Err(Error::Exists { path: path.clone() });
        }
    }
    Ok((socket_path, ledger))
}
