//! One command in a fresh microVM: a run.
//!
//! 0. What the run is given is checked before anything is made: the tree
//!    fits the job drive, the files go under `/job`, the layers are there
//!    and not too many, `--tree-out` and `--upper-out` do not exist yet.
//! 1. The run directory is made; it must not exist yet, or be empty.
//! 2. The job drive, an ext4 file, is built with `mke2fs -d` from
//!    `job.json`, a copy of the tree (or an empty one) and the files. With
//!    an upper drive, an empty ext4 file is made beside its target.
//! 3. `vm.json` is written and Firecracker started on it (`--no-api
//!    --config-file`), in its own process group, with the run directory as
//!    its working directory and the serial console going to `console.log`.
//!    The drives: the image, the job drive, the layers read-only, the upper
//!    drive.
//! 4. kai's init runs the command in the guest and resets the VM, which ends
//!    Firecracker. A VM that has not ended [`GRACE`] after the job's time
//!    limit is killed; `e2fsck -fy` then repairs the job drive.
//! 5. `out/` is read back from the job drive with `debugfs`, which never
//!    mounts it, into files whose names kai chooses; with `--tree-out`, the
//!    tree too (`debugfs rdump`). The upper drive is kept, renamed into
//!    place, only when the command exited with 0; otherwise it is removed.
//!    Neither a layer nor an upper drive is ever mounted or read on the
//!    host.
//! 6. `result.json` is written, last and once.
//!
//! The Firecracker process belongs to the run: dropping the future of
//! [`run()`] kills it. A process killed outright cannot do that; the VM
//! then ends at the guest's own time limit.
//!
//! [`run()`] takes these steps in order; the modules below hold the parts:
//! `spec`, what a run is given; `check`, step 0; `ways_out`, the ports and
//! the egress, open while the VM runs; `boot`, steps 2 to 4; `drives`, the
//! job drive and the upper drive on the host; `result`, `result.json` and
//! the outcome read back.

mod boot;
mod check;
mod drives;
mod result;
mod spec;
mod ways_out;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use result::{ImageRecord, Outcome, ProxyRecord, RESULT_FORMAT, RunResult};
pub use spec::{
    DEFAULT_CPUS, DEFAULT_CWD, DEFAULT_MEMORY_MIB, DEFAULT_PATH, DEFAULT_TIMEOUT_S, JobFile,
    Launch, Origin, Port, PortTarget, RunSpec,
};

use helpers::fs::Part;

use crate::Error;
use crate::fetch::hash_file;

/// How long a VM may run past the job's time limit before it is killed:
/// the boot before the command and the shutdown after it.
pub const GRACE: Duration = Duration::from_secs(15);

/// Runs `spec` with `launch` and returns its result, which is also written
/// to `result.json` in the run directory. An error means there is no run
/// directory to write a result into; every failure after that is an
/// outcome.
pub async fn run(launch: &Launch, spec: &RunSpec) -> Result<RunResult, Error> {
    let disk_mib = check::check(spec)?;
    let dir = &spec.run_dir;
    make_run_dir(dir)?;
    let image_hash = tokio::spawn(hash_file(spec.image.clone()));
    let layer_hashes: Vec<_> = spec
        .layers
        .iter()
        .map(|layer| tokio::spawn(hash_file(layer.clone())))
        .collect();
    let mut result = result::unstarted(launch, spec);

    // The ways out, ready before the VM starts: a relay per port, and the
    // egress. Their sockets go when the run ends, or when it is dropped.
    let _sockets = ways_out::Sockets(dir.clone());
    let ways_out = ways_out::open(dir, spec).await;

    // The upper drive is made beside its target and renamed to it when it
    // is kept.
    let upper = spec.upper_out.as_deref().map(Part::beside);
    let booted = match &ways_out {
        Ok(ways_out) => boot::boot(launch, spec, disk_mib, upper.as_ref(), ways_out).await,
        Err(reason) => Err(reason.clone()),
    };
    if let Ok(ways_out) = ways_out {
        result.proxy = ways_out.close().await;
    }
    ways_out::remove_sockets(dir);
    match booted {
        Err(reason) => result.reason = Some(reason),
        Ok(ended) => {
            result.wall_ms = Some(millis(ended.wall));
            let exit = ended.status;
            if exit.is_none() {
                drives::repair(dir).await;
            }
            result::read_back(dir, exit, &mut result).await;
            // No status, and Firecracker itself failed: the VM did not run.
            // A guest that resets, after its command or after a panic, ends
            // Firecracker with 0.
            if let Some(status) = exit
                && !status.success()
                && result.outcome == Outcome::GuestFailed
            {
                result.outcome = Outcome::LaunchFailed;
                result.output_complete = None;
                result.reason = Some(format!(
                    "Firecracker {status}: {}",
                    boot::failure_line(&dir.join("firecracker.log"))
                ));
            }
        }
    }
    if let Some(target) = &spec.tree_out
        && result.outcome != Outcome::LaunchFailed
    {
        result.tree_out = Some(match drives::tree_out(dir, target).await {
            Ok(()) => true,
            Err(reason) => {
                tracing::warn!(%reason, "the tree was not written out");
                false
            }
        });
    }
    if let Some(upper) = upper {
        let keep = result.outcome == Outcome::Exited && result.exit_code == Some(0);
        result.upper_out = Some(keep && upper.keep().is_ok());
    }
    boot::strip_carriage_returns(&dir.join("console.log"));
    result.image.file = match image_hash.await {
        Ok(Ok(hash)) => Some(result::sha256(&hash)),
        _ => None,
    };
    for hash in layer_hashes {
        if let Ok(Ok(hash)) = hash.await {
            result.layers.push(result::sha256(&hash));
        }
    }
    result::write_result(dir, &result)?;
    Ok(result)
}

/// A run directory under `runs/` in `cache` that does not exist yet:
/// `<seconds since 1970>-<process id>`, then `-<kind>` when one is given,
/// and `-<n>` while that is taken.
pub fn new_run_dir(cache: &Path, kind: Option<&str>) -> PathBuf {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let mut base = format!("{seconds}-{}", std::process::id());
    if let Some(kind) = kind {
        base = format!("{base}-{kind}");
    }
    let runs = cache.join("runs");
    let mut dir = runs.join(&base);
    let mut n = 1;
    while dir.exists() {
        dir = runs.join(format!("{base}-{n}"));
        n += 1;
    }
    dir
}

/// Step 1: the run directory, which must not exist yet or be empty.
fn make_run_dir(dir: &Path) -> Result<(), Error> {
    let io = |source| Error::Io {
        path: dir.to_path_buf(),
        source,
    };
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    match std::fs::create_dir(dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read_dir(dir).map_err(io)?.next().is_some() {
                return Err(io(std::io::Error::other(
                    "the run directory exists and is not empty",
                )));
            }
            Ok(())
        }
        Err(err) => Err(io(err)),
    }
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    std::path::absolute(path).map_err(|err| format!("{}: {err}", path.display()))
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A run of `/probe exit 7` with kai's defaults, for the tests of the
    /// run's modules.
    pub(super) fn spec() -> RunSpec {
        RunSpec::new(
            PathBuf::from("/images/test.sqfs"),
            vec!["/probe".to_owned(), "exit".to_owned(), "7".to_owned()],
            PathBuf::from("/runs/1"),
        )
    }

    /// An empty directory of its own for the test `test`.
    pub(super) fn dir(test: &str) -> helpers::testing::TestDir {
        helpers::testing::TestDir::new(&format!("vm-run-{test}"))
    }

    #[test]
    fn a_run_directory_that_holds_something_is_refused() {
        let dir = dir("run-dir");
        make_run_dir(&dir).unwrap();
        std::fs::write(dir.join("result.json"), b"{}").unwrap();
        assert!(make_run_dir(&dir).is_err());
        make_run_dir(&dir.join("deeper/new")).unwrap();
    }

    #[test]
    fn a_new_run_directory_is_under_runs_named_by_its_kind_and_not_there() {
        let cache = dir("new-run-dir");
        let first = new_run_dir(&cache, Some("fetch"));
        assert_eq!(first.parent(), Some(cache.join("runs").as_path()));
        let name = first.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.ends_with(&format!("-{}-fetch", std::process::id())),
            "{name}"
        );
        std::fs::create_dir_all(&first).unwrap();
        let second = new_run_dir(&cache, Some("fetch"));
        assert_ne!(second, first);
        assert!(!second.exists());
        let plain = new_run_dir(&cache, None);
        let name = plain.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.ends_with(&format!("-{}", std::process::id())),
            "{name}"
        );
    }
}
