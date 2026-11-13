//! What kai's launcher (crates/vm) and its init (the program `kai-init`,
//! PID 1 of every microVM) pass between them on the job drive: the job the
//! init runs, [`Job`] in `job.json`, and how the command ended, [`Status`]
//! in `out/status.json`. The two ship together in one kai, so these files
//! carry a format number that the reader checks, and no page in
//! `contracts/`.
//!
//! The job drive, as the launcher writes it and the guest sees it at
//! `/job`:
//!
//! ```text
//! job.json      the job
//! tree/         the work tree; the command's directory by default
//! out/stdout    what the command wrote to standard output
//! out/stderr    what it wrote to standard error; the init's own line too
//!               when the command did not start
//! out/status.json  how it ended, written by the init after the command
//! ...           files the launcher was given for `/job`, beside the tree
//! upper/, work/ the overlay's upper layer, unless it has a drive of its own
//! ```
//!
//! The command runs as the job's user, root unless the job says otherwise.
//! Root in the guest can change anything on the job drive, its status
//! included; that tells nothing it could not say by its exit status.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The format of `job.json` this init reads.
pub const JOB_FORMAT: u32 = 1;

/// The format of `status.json` this init writes.
pub const STATUS_FORMAT: u32 = 1;

/// The job, relative to the job drive's root.
pub const JOB_FILE: &str = "job.json";
/// The work tree, relative to the job drive's root.
pub const TREE_DIR: &str = "tree";
/// The command's output and the status, relative to the job drive's root.
pub const OUT_DIR: &str = "out";
/// Standard output, relative to [`OUT_DIR`].
pub const STDOUT_FILE: &str = "stdout";
/// Standard error, relative to [`OUT_DIR`].
pub const STDERR_FILE: &str = "stderr";
/// The status, relative to [`OUT_DIR`].
pub const STATUS_FILE: &str = "status.json";

/// What the init runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    /// [`JOB_FORMAT`]; the init refuses another.
    pub format: u32,
    /// The program and its arguments. The program is looked up on the
    /// `PATH` of `env` unless it holds a `/`.
    pub argv: Vec<String>,
    /// The command's whole environment; nothing else is passed on.
    pub env: BTreeMap<String, String>,
    /// The command's directory in the guest, `/job/tree` by default.
    pub cwd: String,
    /// The user id the command runs as.
    pub uid: u32,
    /// The group id the command runs as.
    pub gid: u32,
    /// Seconds from the command's start until the init ends it.
    pub timeout_s: u64,
    /// How many read-only layers lie between the image and the upper
    /// layer: the drives after the job drive, `/dev/vdc` on, the last one
    /// given on top.
    #[serde(default)]
    pub layers: u32,
    /// Whether the overlay's upper layer goes to a drive of its own, the
    /// last drive, instead of to the job drive; the launcher keeps that
    /// drive as a layer.
    #[serde(default)]
    pub upper_out: bool,
    /// The ports: for each, the init listens on the guest's
    /// `127.0.0.1:<port>` and passes every connection on to vsock, the host
    /// (context id 2), the same port.
    #[serde(default)]
    pub ports: Vec<u16>,
}

/// The device of the drive at `index` in `vm.json`'s order: 0 is
/// `/dev/vda`, the image; 1 the job drive; then the layers; then the upper
/// drive.
pub fn drive(index: u32) -> String {
    let letter = char::from(b'a' + u8::try_from(index).unwrap_or(25).min(25));
    format!("/dev/vd{letter}")
}

/// The most layers a run takes; the drives run out at `/dev/vdz`.
pub const MAX_LAYERS: u32 = 8;

/// How the command ended, as the init saw it. (Not `deny_unknown_fields`:
/// serde does not take it together with `flatten`; the format number is
/// the check.)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// [`STATUS_FORMAT`]; the launcher refuses another.
    pub format: u32,
    /// How it ended.
    #[serde(flatten)]
    pub ending: Ending,
    /// Milliseconds from the command's start to its end, measured in the
    /// guest.
    pub command_ms: u64,
}

/// How a command ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum Ending {
    /// It ended by itself with an exit status. A program that could not be
    /// started ends as a shell says: 127 when it is not there, 126 when it
    /// cannot be run.
    Exited {
        /// The exit status.
        exit_code: i32,
    },
    /// A signal ended it.
    Signalled {
        /// The signal's number.
        signal: i32,
    },
    /// The init ended it at the job's time limit.
    Timeout,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_is_flat_json() {
        let status = Status {
            format: STATUS_FORMAT,
            ending: Ending::Exited { exit_code: 7 },
            command_ms: 310,
        };
        let text = serde_json::to_string(&status).unwrap();
        assert_eq!(
            text,
            r#"{"format":1,"outcome":"exited","exit_code":7,"command_ms":310}"#
        );
        assert_eq!(serde_json::from_str::<Status>(&text).unwrap(), status);

        let timeout: Status =
            serde_json::from_str(r#"{"format":1,"outcome":"timeout","command_ms":2000}"#).unwrap();
        assert_eq!(timeout.ending, Ending::Timeout);
    }

    #[test]
    fn drives_are_named_in_order() {
        assert_eq!(drive(0), "/dev/vda");
        assert_eq!(drive(1), "/dev/vdb");
        assert_eq!(drive(2 + MAX_LAYERS), "/dev/vdk");
    }

    #[test]
    fn a_job_with_a_key_the_init_does_not_know_is_refused() {
        let text = r#"{"format":1,"argv":["true"],"env":{},"cwd":"/job/tree","uid":0,"gid":0,"timeout_s":600,"network":true}"#;
        assert!(serde_json::from_str::<Job>(text).is_err());
    }
}
