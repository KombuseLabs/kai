//! `fence proxy` as a child process: the command line kai starts it with,
//! the line that says it listens, its standard error, and its end. Each is
//! a part of the contract the crate's documentation lists.

use std::io;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use helpers::process::{Kept, keep_start, wait_or_kill};
use tokio::io::{AsyncBufReadExt as _, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;

use crate::ledger::LEDGER;
use crate::{Error, Policy};

/// How much of fence's standard error is kept for an error message.
const STDERR_KEPT: usize = 16 * 1024;

/// How much of an unexpected first line is quoted.
const LINE_QUOTED: usize = 200;

/// A `fence proxy` that has said it listens. Dropping it kills fence.
#[derive(Debug)]
pub(crate) struct Fence {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: JoinHandle<Kept>,
    stdout: JoinHandle<()>,
}

impl Fence {
    /// Starts fence in `dir` on the socket `socket` with `policy`'s rules,
    /// and returns once it says it listens. A fence that ends first, says
    /// something else or nothing in time is an error, and is not left
    /// running.
    pub(crate) async fn start(dir: &Path, socket: &str, policy: &Policy) -> Result<Fence, Error> {
        let mut child =
            command(dir, socket, policy)
                .spawn()
                .map_err(|source| match source.kind() {
                    io::ErrorKind::NotFound => Error::FenceMissing {
                        program: policy.fence.clone(),
                    },
                    _ => Error::Io {
                        what: "cannot start",
                        path: policy.fence.clone(),
                        source,
                    },
                })?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            unreachable!("all three streams are piped");
        };
        let stderr = keep_start(stderr, STDERR_KEPT);

        let mut lines = BufReader::new(stdout).lines();
        let ready = format!("listening unix:{socket}");
        match tokio::time::timeout(policy.ready_timeout, lines.next_line()).await {
            Ok(Ok(Some(line))) if line == ready => {}
            Ok(Ok(Some(line))) => {
                let _ = child.kill().await;
                return Err(Error::UnexpectedLine {
                    line: line.chars().take(LINE_QUOTED).collect(),
                });
            }
            Ok(Ok(None) | Err(_)) => {
                // Its standard output closed: fence is ending.
                let how = match wait_or_kill(&mut child, policy.close_grace).await {
                    Ok(Some(status)) => status.to_string(),
                    Ok(None) => "it closed its output and did not end, so it was killed".to_owned(),
                    Err(e) => format!("its status could not be read: {e}"),
                };
                return Err(Error::Refused {
                    how,
                    stderr: stderr.await?.text(),
                });
            }
            Err(_elapsed) => {
                let _ = child.kill().await;
                return Err(Error::NotReady {
                    after: policy.ready_timeout,
                    stderr: stderr.await?.text(),
                });
            }
        }
        Ok(Fence {
            child,
            stdin: Some(stdin),
            stderr,
            stdout: tokio::spawn(drain(lines)),
        })
    }

    /// Closes fence's standard input, which ends it, and waits up to
    /// `grace`; a fence still running then is killed. Fine only when it
    /// ended with 0; its output is read to the end either way.
    pub(crate) async fn end(mut self, grace: Duration) -> Result<(), Error> {
        drop(self.stdin.take());
        let clean = match wait_or_kill(&mut self.child, grace).await {
            Ok(Some(status)) if status.success() => Ok(()),
            Ok(Some(status)) => Err(status.to_string()),
            Ok(None) => Err(format!(
                "still running {grace:?} after its input closed, so it was killed"
            )),
            Err(e) => Err(format!("its status could not be read: {e}")),
        };
        let stderr = self.stderr.await?.text();
        self.stdout.await?;
        if !stderr.is_empty() {
            tracing::debug!(%stderr, "fence proxy ended");
        }
        clean.map_err(|how| Error::Ended { how, stderr })
    }
}

/// The command line of the contract: `fence proxy --listen=unix:<socket>
/// --net=<rule>… --ledger=proxy.jsonl [--learn]` in `dir`, with `PATH` and
/// `HOME` as its only environment, its three streams piped and killed when
/// dropped. The `=` keeps a rule from being read as an option.
fn command(dir: &Path, socket: &str, policy: &Policy) -> Command {
    let mut command = Command::new(&policy.fence);
    command
        .arg("proxy")
        .arg(format!("--listen=unix:{socket}"))
        .args(policy.net.iter().map(|rule| format!("--net={rule}")))
        .arg(format!("--ledger={LEDGER}"));
    if policy.learn {
        command.arg("--learn");
    }
    command.current_dir(dir).env_clear();
    for name in ["PATH", "HOME"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

/// Reads what fence writes after its ready line, which the contract says
/// is nothing, and notes it.
async fn drain(mut lines: Lines<BufReader<ChildStdout>>) {
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::warn!(%line, "fence proxy wrote to standard output after its ready line");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_is_the_agreed_one() {
        let policy = Policy {
            learn: true,
            fence: "/opt/fence/bin/fence".into(),
            ..Policy::new(["index.crates.io:443", "*.example.org:443"])
        };
        let command = command(Path::new("/runs/r1"), "v.sock_3128", &policy);
        let command = command.as_std();
        assert_eq!(command.get_program(), "/opt/fence/bin/fence");
        let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "proxy",
                "--listen=unix:v.sock_3128",
                "--net=index.crates.io:443",
                "--net=*.example.org:443",
                "--ledger=proxy.jsonl",
                "--learn",
            ]
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/runs/r1")));
        for (name, _) in command.get_envs() {
            assert!(name == "PATH" || name == "HOME", "{name:?}");
        }
    }
}
