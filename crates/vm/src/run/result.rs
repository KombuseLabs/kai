//! What a run returns: `result.json` (`contracts/kai-vm-run-1.md`), the
//! outcome made from what the guest left on the job drive (step 5), and the
//! file written last (step 6).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{ExitStatus, Stdio};

use guest::{Ending, OUT_DIR, STATUS_FILE, STATUS_FORMAT, STDERR_FILE, STDOUT_FILE, Status};
use helpers::fs::write_whole;
use helpers::process;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::{GRACE, Launch, RunSpec};
use crate::Error;

/// The format `result.json` names.
pub const RESULT_FORMAT: &str = "kai-vm-run/1";

/// The largest `status.json` read back.
const MAX_STATUS: u64 = 64 * 1024;

/// The image of a run, as `result.json` records it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRecord {
    /// The recipe, for an image made from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    /// The image's id, for an image made from a recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// SHA-256 of the image file, as `sha256:<hex>`, when it could be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// The command ended by itself; `exit_code` says how.
    Exited,
    /// A signal ended the command; `signal` names it.
    Signalled,
    /// The guest ended the command at its time limit.
    Timeout,
    /// The VM did not end in time and kai killed it.
    Killed,
    /// The VM ended without a status: the init or the kernel failed;
    /// `console.log` says more.
    GuestFailed,
    /// The VM did not start; `reason` says why.
    LaunchFailed,
}

impl Outcome {
    /// The outcome as `result.json` names it.
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Exited => "exited",
            Outcome::Signalled => "signalled",
            Outcome::Timeout => "timeout",
            Outcome::Killed => "killed",
            Outcome::GuestFailed => "guest-failed",
            Outcome::LaunchFailed => "launch-failed",
        }
    }

    /// Whether the command decided the outcome. The other three are
    /// failures of the infrastructure, never of the command.
    pub fn is_the_commands(self) -> bool {
        matches!(
            self,
            Outcome::Exited | Outcome::Signalled | Outcome::Timeout
        )
    }
}

/// `result.json`: how a run went (`contracts/kai-vm-run-1.md`). A value
/// nobody measured is absent, never 0.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunResult {
    /// [`RESULT_FORMAT`].
    pub format: String,
    /// How the run ended.
    pub outcome: Outcome,
    /// The command's exit status; for `exited` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// The signal that ended the command; for `signalled` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    /// Why the run failed, for `killed`, `guest-failed` and
    /// `launch-failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Whether `out/stdout` and `out/stderr` hold all the command wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_complete: Option<bool>,
    /// The image: its recipe and id when it was made from one, and the
    /// hash of its file.
    pub image: ImageRecord,
    /// SHA-256 of the kernel.
    pub kernel: String,
    /// SHA-256 of the Firecracker program.
    pub firecracker: String,
    /// SHA-256 of the initramfs, and so of kai's init.
    pub initramfs: String,
    /// Processors of the VM.
    pub cpus: u32,
    /// Memory of the VM, in MiB.
    pub memory_mib: u32,
    /// The command's time limit, in seconds.
    pub timeout_s: u64,
    /// Milliseconds from Firecracker's start to its end, on the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_ms: Option<u64>,
    /// Milliseconds the command ran, measured in the guest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_ms: Option<u64>,
    /// The layers over the image, bottom first, as `sha256:<hex>` of each
    /// file; absent without layers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<String>,
    /// Whether the upper drive was kept as a layer; absent when none was
    /// asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper_out: Option<bool>,
    /// Whether the tree was written to `--tree-out`; absent when it was not
    /// asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_out: Option<bool>,
    /// What went through the egress, when the run had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyRecord>,
}

/// What went through a run's egress, counted from its ledger,
/// `proxy.jsonl` in the run directory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyRecord {
    /// Connections let through.
    pub allowed: u64,
    /// Connections refused.
    pub refused: u64,
    /// Each `host:port` a refused connection asked for, with how often.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refused_targets: BTreeMap<String, u64>,
    /// In learn mode, connections let through that no rule names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub would_deny: Option<u64>,
    /// In learn mode, each `host:port` no rule names, with how often.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub would_deny_targets: BTreeMap<String, u64>,
}

/// A hash as `result.json` writes it, `sha256:<hex>`.
pub(super) fn sha256(hash: &str) -> String {
    format!("sha256:{hash}")
}

/// The result before the VM ran: `launch-failed` until the run knows
/// better, with what the launch and the spec say.
pub(super) fn unstarted(launch: &Launch, spec: &RunSpec) -> RunResult {
    RunResult {
        format: RESULT_FORMAT.to_owned(),
        outcome: Outcome::LaunchFailed,
        exit_code: None,
        signal: None,
        reason: None,
        output_complete: None,
        image: ImageRecord {
            recipe: spec.origin.as_ref().map(|origin| origin.recipe.clone()),
            id: spec.origin.as_ref().map(|origin| origin.id.clone()),
            file: None,
        },
        kernel: sha256(&launch.kernel_sha256),
        firecracker: sha256(&launch.firecracker_sha256),
        initramfs: sha256(&launch.initramfs_sha256),
        cpus: spec.cpus,
        memory_mib: spec.memory_mib,
        timeout_s: spec.timeout_s,
        wall_ms: None,
        command_ms: None,
        layers: Vec::new(),
        upper_out: None,
        tree_out: None,
        proxy: None,
    }
}

/// Step 5: `out/status.json`, `out/stdout` and `out/stderr` from the job
/// drive into the run directory's `out/`, and the outcome they make.
pub(super) async fn read_back(dir: &Path, exit: Option<ExitStatus>, result: &mut RunResult) {
    let killed = exit.is_none();
    result.outcome = if killed {
        Outcome::Killed
    } else {
        Outcome::GuestFailed
    };
    result.output_complete = Some(false);
    result.reason = Some(if killed {
        format!(
            "the VM did not end within {} s after the time limit",
            GRACE.as_secs()
        )
    } else {
        "the VM ended without a status; console.log says more".to_owned()
    });

    if let Err(err) = std::fs::create_dir(dir.join(OUT_DIR)) {
        result.reason = Some(format!("make out/: {err}"));
        return;
    }
    // One request each, in the run directory: `dump` writes a plain file
    // whatever the guest left there. debugfs ends with 0 when a file is not
    // on the drive; read_status sees that.
    for name in [STATUS_FILE, STDOUT_FILE, STDERR_FILE] {
        let mut command = Command::new("debugfs");
        command
            .arg("-R")
            .arg(format!("dump /{OUT_DIR}/{name} {OUT_DIR}/{name}"))
            .arg("job.ext4")
            .current_dir(dir)
            .stdin(Stdio::null());
        if let Err(failed) = process::run(&mut command).await {
            result.reason = Some(failed.to_string());
            return;
        }
    }

    let status = match read_status(&dir.join(OUT_DIR).join(STATUS_FILE)) {
        Ok(Some(status)) => status,
        Ok(None) => return,
        Err(reason) => {
            result.reason = Some(reason);
            return;
        }
    };
    if killed {
        // The guest wrote a status, but the VM still had to be killed.
        return;
    }
    result.reason = None;
    result.output_complete = Some(true);
    result.command_ms = Some(status.command_ms);
    match status.ending {
        Ending::Exited { exit_code } => {
            result.outcome = Outcome::Exited;
            result.exit_code = Some(exit_code);
        }
        Ending::Signalled { signal } => {
            result.outcome = Outcome::Signalled;
            result.signal = Some(signal);
        }
        Ending::Timeout => result.outcome = Outcome::Timeout,
    }
}

/// The status the init wrote, if it wrote one. What came back from the
/// guest is data: a status that is not a plain file of sane size, or not
/// the format this kai reads, is refused.
fn read_status(path: &Path) -> Result<Option<Status>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("read status.json: {err}")),
    };
    if !meta.file_type().is_file() || meta.len() > MAX_STATUS {
        return Err("status.json is not a plain file of at most 64 KiB".to_owned());
    }
    let text = std::fs::read(path).map_err(|err| format!("read status.json: {err}"))?;
    let status: Status =
        serde_json::from_slice(&text).map_err(|err| format!("read status.json: {err}"))?;
    if status.format != STATUS_FORMAT {
        return Err(format!(
            "status.json has format {}; this kai reads {STATUS_FORMAT}",
            status.format
        ));
    }
    Ok(Some(status))
}

/// Step 6: `result.json`, under a temporary name first, so that a reader
/// never sees half of it.
pub(super) fn write_result(dir: &Path, result: &RunResult) -> Result<(), Error> {
    let path = dir.join("result.json");
    let io = |source| Error::Io {
        path: path.clone(),
        source,
    };
    let mut text = serde_json::to_vec_pretty(result).map_err(|err| io(err.into()))?;
    text.push(b'\n');
    write_whole(&path, &text).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::tests::dir;
    use crate::{FIRECRACKER, KERNEL};

    #[test]
    fn a_status_is_read_only_from_a_plain_file_of_this_format() {
        let dir = dir("status");
        let path = dir.join("status.json");
        assert_eq!(read_status(&path), Ok(None));

        std::fs::write(
            &path,
            r#"{"format":1,"outcome":"exited","exit_code":7,"command_ms":3}"#,
        )
        .unwrap();
        let status = read_status(&path).unwrap().unwrap();
        assert_eq!(status.ending, Ending::Exited { exit_code: 7 });

        std::fs::write(&path, r#"{"format":2,"outcome":"timeout","command_ms":3}"#).unwrap();
        assert!(read_status(&path).unwrap_err().contains("format 2"));

        // What came back from the guest may point anywhere on this machine.
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("/etc/hostname", &path).unwrap();
        assert!(read_status(&path).unwrap_err().contains("not a plain file"));
    }

    /// contracts/kai-vm-run-1.example.json is what kai writes.
    #[test]
    fn the_contracts_example_is_what_kai_writes() {
        let example = include_str!("../../../../contracts/kai-vm-run-1.example.json");
        let result = RunResult {
            format: RESULT_FORMAT.to_owned(),
            outcome: Outcome::Exited,
            exit_code: Some(7),
            signal: None,
            reason: None,
            output_complete: Some(true),
            image: ImageRecord {
                recipe: Some("rust".to_owned()),
                id: Some(format!("sha256:{}", "1".repeat(64))),
                file: Some(format!("sha256:{}", "3".repeat(64))),
            },
            kernel: format!("sha256:{}", KERNEL.sha256),
            firecracker: format!("sha256:{}", FIRECRACKER.sha256),
            initramfs: format!("sha256:{}", "2".repeat(64)),
            cpus: 2,
            memory_mib: 2048,
            timeout_s: 600,
            wall_ms: Some(710),
            command_ms: Some(3),
            layers: Vec::new(),
            upper_out: None,
            tree_out: None,
            proxy: None,
        };
        let mut written = serde_json::to_string_pretty(&result).unwrap();
        written.push('\n');
        assert_eq!(written, example);
        assert_eq!(serde_json::from_str::<RunResult>(example).unwrap(), result);
    }
}
