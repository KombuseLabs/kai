//! Child processes. Every crate that runs a program starts from here, so
//! that a tool's failure reads the same everywhere and no child ever
//! blocks on a full pipe.
//!
//! - [`run`]: a tool run to its end; its standard output, or how it failed
//!   with its standard error.
//! - [`keep_start`]: the start of a stream of a program that runs longer,
//!   read to its end in a task of its own.
//! - [`wait_or_kill`]: waits for a child up to a limit, and kills it then.
//!
//! What is particular to one program (its command line, a line it says
//! when it is ready, its process group) stays with the crate that runs it.
//! Async, on tokio; the program that calls these runs the runtime.

use std::io;
use std::process::ExitStatus;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

/// How a tool failed.
#[derive(Debug, thiserror::Error)]
pub enum Failed {
    /// It could not be started: not found, or not executable.
    #[error("cannot run {program}: {source}")]
    Start {
        /// The program, as the command names it.
        program: String,
        /// What the system said.
        #[source]
        source: io::Error,
    },
    /// It ran and ended otherwise than with 0.
    #[error("{program} {status}: {stderr}")]
    Status {
        /// The program, as the command names it.
        program: String,
        /// How it ended.
        status: ExitStatus,
        /// Its standard error, trimmed.
        stderr: String,
    },
}

/// Runs `command` to its end and returns its standard output. Its standard
/// input is what the command sets, else none. A program that cannot be
/// started, or ends otherwise than with 0, is a [`Failed`] that carries
/// its standard error.
///
/// ```
/// # #[tokio::main(flavor = "current_thread")]
/// # async fn main() {
/// use helpers::process::{Failed, run};
/// use tokio::process::Command;
///
/// let out = run(Command::new("sh").args(["-c", "echo hello"])).await.unwrap();
/// assert_eq!(out, b"hello\n");
///
/// let failed = run(Command::new("sh").args(["-c", "echo no >&2; exit 3"]))
///     .await
///     .unwrap_err();
/// assert!(matches!(&failed, Failed::Status { stderr, .. } if stderr == "no"));
/// assert_eq!(failed.to_string(), "sh exit status: 3: no");
/// # }
/// ```
pub async fn run(command: &mut Command) -> Result<Vec<u8>, Failed> {
    let program = command
        .as_std()
        .get_program()
        .to_string_lossy()
        .into_owned();
    let output = command.output().await.map_err(|source| Failed::Start {
        program: program.clone(),
        source,
    })?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(Failed::Status {
        program,
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

/// The start of what a program wrote, as [`keep_start`] keeps it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    /// The first bytes, up to the limit.
    pub bytes: Vec<u8>,
    /// How many bytes came after them and were dropped.
    pub more: u64,
}

impl Kept {
    /// The kept bytes as text, trimmed, with a note of what was dropped.
    pub fn text(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.bytes).trim().to_owned();
        if self.more > 0 {
            text.push_str(&format!(" ({} more bytes)", self.more));
        }
        text
    }
}

/// Reads `stream` to its end in a task of its own, keeping its first
/// `limit` bytes and counting the rest, so that the program writing it
/// never blocks on a full pipe. A read that fails ends it with what was
/// read until then.
pub fn keep_start<R>(stream: R, limit: usize) -> JoinHandle<Kept>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut stream = stream;
        let mut kept = Kept::default();
        let mut chunk = [0u8; 8192];
        loop {
            let n = match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let take = n.min(limit.saturating_sub(kept.bytes.len()));
            kept.bytes.extend_from_slice(&chunk[..take]);
            kept.more += (n - take) as u64;
        }
        kept
    })
}

/// Waits for `child` up to `limit`. A child still running then is killed
/// and reaped, and `None` says so.
pub async fn wait_or_kill(child: &mut Child, limit: Duration) -> io::Result<Option<ExitStatus>> {
    match tokio::time::timeout(limit, child.wait()).await {
        Ok(status) => status.map(Some),
        Err(_elapsed) => {
            // A child that ended in the moment between is reaped all the
            // same; either way it runs no more.
            let _ = child.kill().await;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Stdio;

    use super::*;

    #[tokio::test]
    async fn a_program_that_is_not_there_is_a_start_failure() {
        let failed = run(&mut Command::new("kai-no-such-program"))
            .await
            .unwrap_err();
        assert!(
            matches!(&failed, Failed::Start { program, .. } if program == "kai-no-such-program"),
            "{failed}"
        );
    }

    #[tokio::test]
    async fn run_keeps_the_input_the_command_sets() {
        let mut command = Command::new("cat");
        command.stdin(Stdio::null());
        assert_eq!(run(&mut command).await.unwrap(), b"");
    }

    #[tokio::test]
    async fn keep_start_keeps_the_limit_and_counts_the_rest() {
        let kept = keep_start(io::Cursor::new(b"0123456789".to_vec()), 4)
            .await
            .unwrap();
        assert_eq!(kept.bytes, b"0123");
        assert_eq!(kept.more, 6);
        assert_eq!(kept.text(), "0123 (6 more bytes)");
        let all = keep_start(io::Cursor::new(b" all of it \n".to_vec()), 64)
            .await
            .unwrap();
        assert_eq!((all.text().as_str(), all.more), ("all of it", 0));
    }

    #[tokio::test]
    async fn keep_start_drains_a_child_that_writes_more_than_a_pipe_holds() {
        let mut child = Command::new("sh")
            .args(["-c", "head -c 200000 /dev/zero >&2; echo done"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = keep_start(child.stderr.take().unwrap(), 1024);
        let out = child.wait_with_output().await.unwrap();
        assert_eq!(out.stdout, b"done\n");
        let kept = stderr.await.unwrap();
        assert_eq!((kept.bytes.len(), kept.more), (1024, 200_000 - 1024));
    }

    #[tokio::test]
    async fn wait_or_kill_waits_for_an_end_and_kills_past_the_limit() {
        let mut quick = Command::new("true").spawn().unwrap();
        let ended = wait_or_kill(&mut quick, Duration::from_secs(5)).await;
        assert!(ended.unwrap().unwrap().success());

        let mut slow = Command::new("sleep").arg("30").spawn().unwrap();
        let started = std::time::Instant::now();
        let ended = wait_or_kill(&mut slow, Duration::from_millis(100)).await;
        assert!(ended.unwrap().is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(slow.try_wait().unwrap().is_some(), "killed and reaped");
    }
}
