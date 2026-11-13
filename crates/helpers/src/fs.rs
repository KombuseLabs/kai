//! Files put in place whole. A file is written, or made by a program, as
//! a [`Part`] beside its target and renamed to the target only once it is
//! complete: a reader finds the old file or the new one, never half of
//! one, and a failure leaves nothing behind. A directory is made the same
//! way, as a [`PartDir`]. Where more than one process may put the same
//! target, a [`Lock`] keeps them apart; holding it, [`remove_parts`] clears
//! what a killed process left, and [`sync_dir`] makes a rename outlive a
//! crash.
//!
//! ```
//! use helpers::fs::{Part, write_whole};
//!
//! let dir = std::env::temp_dir().join(format!("helpers-fs-doc-{}", std::process::id()));
//! std::fs::create_dir_all(&dir)?;
//! let target = dir.join("settings.json");
//! write_whole(&target, b"{}\n")?;
//!
//! // A part that is not kept goes, and the target is as it was.
//! let part = Part::beside(&target);
//! std::fs::write(part.path(), b"{\"half")?;
//! drop(part);
//! assert_eq!(std::fs::read(&target)?, b"{}\n");
//! assert_eq!(std::fs::read_dir(&dir)?.count(), 1);
//! # std::fs::remove_dir_all(&dir)?;
//! # Ok::<(), std::io::Error>(())
//! ```

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `bytes` to `path` whole, through a [`Part`]; a file at `path` is
/// replaced.
pub fn write_whole(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let part = Part::beside(path);
    std::fs::write(part.path(), bytes)?;
    part.keep()
}

/// A file on its way to its target, in the target's directory, so that
/// putting it in place is a rename. Its name, `.<target>.<process>-<n>.part`,
/// is its own among processes and within one. Dropped without
/// [`Part::keep`], or when keeping it fails, it is removed.
#[derive(Debug)]
pub struct Part {
    path: PathBuf,
    target: PathBuf,
}

/// The name a part of `target` starts with, `.<target>.`, before the
/// process, the number and `.part`.
fn part_prefix(target: &Path) -> String {
    let name = target.file_name().map_or_else(
        || "file".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    format!(".{name}.")
}

/// A new part's path for `target`: `.<target>.<process>-<n>.part`, its own
/// among processes and within one, for files and directories alike.
fn part_path(target: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(
        "{}{}-{n}.part",
        part_prefix(target),
        std::process::id()
    ))
}

impl Part {
    /// The part for `target`. Nothing is made yet.
    pub fn beside(target: &Path) -> Part {
        Part {
            path: part_path(target),
            target: target.to_path_buf(),
        }
    }

    /// Where the part lies, to be written or made there.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Renames the part to its target, replacing what is there.
    pub fn keep(mut self) -> io::Result<()> {
        std::fs::rename(&self.path, &self.target)?;
        self.path = PathBuf::new();
        Ok(())
    }
}

impl Drop for Part {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A directory on its way to its target, the twin of [`Part`]: made in the
/// target's directory under a name of the same form, so that putting it in
/// place is a rename. Dropped without [`PartDir::keep`], or when keeping it
/// fails, it is removed with everything in it.
#[derive(Debug)]
pub struct PartDir {
    path: PathBuf,
    target: PathBuf,
}

impl PartDir {
    /// Makes the directory for `target`, empty.
    pub fn beside(target: &Path) -> io::Result<PartDir> {
        let path = part_path(target);
        std::fs::create_dir(&path)?;
        Ok(PartDir {
            path,
            target: target.to_path_buf(),
        })
    }

    /// Where the directory lies, to be filled.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Renames the directory to its target. As `rename(2)` does, it
    /// replaces an empty directory there and fails on one that holds
    /// something; a caller that must replace nothing holds a [`Lock`] and
    /// found nothing at the target.
    pub fn keep(mut self) -> io::Result<()> {
        std::fs::rename(&self.path, &self.target)?;
        self.path = PathBuf::new();
        Ok(())
    }
}

impl Drop for PartDir {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Removes what [`Part`]s and [`PartDir`]s of `target` were left with when
/// their process was killed, which no drop removed: the files and whole
/// directories beside `target` named `.<target>.<process>-<n>.part`, and
/// says how many. Nothing else is touched, a part of another target
/// included. Call it only while holding a [`Lock`] that every writer of
/// `target` takes: otherwise it removes a part that is still being written.
pub fn remove_parts(target: &Path) -> io::Result<usize> {
    let dir = match target.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let prefix = part_prefix(target);
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(middle) = name
            .strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix(".part"))
        else {
            continue;
        };
        let numbers = middle.split_once('-').is_some_and(|(process, n)| {
            [process, n]
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        });
        if !numbers {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
        tracing::info!(part = %path.display(), "removed a part a killed process left");
        removed += 1;
    }
    Ok(removed)
}

/// An exclusive lock on a file, among processes and within one:
/// `flock(2)`, through std's `File::lock`. It is held until the value is
/// dropped, and the system lets go of it when its process ends, killed or
/// not, so a lock is never left behind; the file itself stays.
#[derive(Debug)]
pub struct Lock {
    _file: std::fs::File,
}

impl Lock {
    /// Takes the lock on the file at `path`, which is made when missing.
    /// While another holds it, waits on one of the runtime's blocking
    /// threads, so that no other task is held up, and says so in a log
    /// line.
    pub async fn acquire(path: &Path) -> io::Result<Lock> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => return Ok(Lock { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(err)) => return Err(err),
        }
        tracing::info!(file = %path.display(), "another process holds the lock; waiting for it");
        tokio::task::spawn_blocking(move || file.lock().map(|()| Lock { _file: file }))
            .await
            .unwrap_or_else(|join| Err(io::Error::other(join)))
    }
}

/// Flushes the directory at `path` to the disk, so that the names made,
/// renamed or removed in it outlive a crash of the machine.
pub fn sync_dir(path: &Path) -> io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("helpers-fs-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_whole_write_replaces_the_file_and_leaves_no_part() {
        let dir = dir("replace");
        let target = dir.join("a.json");
        write_whole(&target, b"old").unwrap();
        write_whole(&target, b"new").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(names(&dir), ["a.json"]);
    }

    #[test]
    fn a_part_is_kept_by_a_rename_and_removed_otherwise() {
        let dir = dir("keep");
        let target = dir.join("image");
        let kept = Part::beside(&target);
        let dropped = Part::beside(&target);
        assert_ne!(kept.path(), dropped.path());
        assert!(kept.path().starts_with(&dir));
        std::fs::write(kept.path(), b"kept").unwrap();
        std::fs::write(dropped.path(), b"dropped").unwrap();
        kept.keep().unwrap();
        drop(dropped);
        assert_eq!(std::fs::read(&target).unwrap(), b"kept");
        assert_eq!(names(&dir), ["image"]);
    }

    #[test]
    fn a_part_dir_is_kept_by_a_rename_and_removed_whole_otherwise() {
        let dir = dir("part-dir");
        let target = dir.join("entry");
        let kept = PartDir::beside(&target).unwrap();
        let dropped = PartDir::beside(&target).unwrap();
        assert_ne!(kept.path(), dropped.path());
        assert!(kept.path().starts_with(&dir) && kept.path().is_dir());
        std::fs::write(kept.path().join("layer"), b"kept").unwrap();
        std::fs::create_dir(dropped.path().join("deep")).unwrap();
        std::fs::write(dropped.path().join("deep/layer"), b"dropped").unwrap();
        kept.keep().unwrap();
        drop(dropped);
        assert_eq!(std::fs::read(target.join("layer")).unwrap(), b"kept");
        assert_eq!(names(&dir), ["entry"]);

        // A target that holds something is not replaced, and the part goes.
        let refused = PartDir::beside(&target).unwrap();
        assert!(refused.keep().is_err());
        assert_eq!(names(&dir), ["entry"]);
        assert_eq!(std::fs::read(target.join("layer")).unwrap(), b"kept");
    }

    #[test]
    fn remove_parts_removes_the_targets_parts_and_nothing_else() {
        let dir = dir("remove-parts");
        let target = dir.join("claude-1.0");
        std::fs::write(dir.join(".claude-1.0.12-0.part"), b"half").unwrap();
        std::fs::create_dir_all(dir.join(".claude-1.0.12-1.part/stage")).unwrap();
        for not_a_part in [
            "claude-1.0",
            ".claude-1.0.lock",
            ".claude-1.0.x.12-0.part",
            ".claude-1.0.12.part",
            ".claude-1.0.12-.part",
            ".claude-1.0.a-0.part",
            ".claude-1.0.quarantine-1-2-3",
        ] {
            std::fs::write(dir.join(not_a_part), b"stays").unwrap();
        }
        assert_eq!(remove_parts(&target).unwrap(), 2);
        assert_eq!(
            names(&dir),
            [
                ".claude-1.0.12-.part",
                ".claude-1.0.12.part",
                ".claude-1.0.a-0.part",
                ".claude-1.0.lock",
                ".claude-1.0.quarantine-1-2-3",
                ".claude-1.0.x.12-0.part",
                "claude-1.0",
            ]
        );
        assert_eq!(remove_parts(&target).unwrap(), 0);
        assert_eq!(remove_parts(&dir.join("missing/target")).unwrap(), 0);
    }

    /// The child of the next test: holds a part dir and a part, says so,
    /// and waits to be killed. Run alone, without the variable, it does
    /// nothing.
    #[test]
    #[ignore = "the child of parts_a_killed_process_left_are_removed_and_nothing_else"]
    fn a_child_that_holds_parts_until_it_is_killed() {
        let Some(dir) = std::env::var_os("KAI_HELPERS_PART_CHILD").map(PathBuf::from) else {
            return;
        };
        let entry = PartDir::beside(&dir.join("entry")).unwrap();
        std::fs::write(entry.path().join("layer"), b"half a layer").unwrap();
        let download = Part::beside(&dir.join("download"));
        std::fs::write(download.path(), b"half a download").unwrap();
        write_whole(&dir.join("ready"), b"").unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }

    #[test]
    fn parts_a_killed_process_left_are_removed_and_nothing_else() {
        let dir = dir("killed");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "fs::tests::a_child_that_holds_parts_until_it_is_killed",
                "--ignored",
                "--test-threads=1",
            ])
            .env("KAI_HELPERS_PART_CHILD", &dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        while !dir.join("ready").exists() {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(30),
                "the child never held its parts"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // SIGKILL: no drop runs, so both parts stay.
        child.kill().unwrap();
        child.wait().unwrap();
        let parts: Vec<String> = names(&dir)
            .into_iter()
            .filter(|name| name.ends_with(".part"))
            .collect();
        assert_eq!(parts.len(), 2, "{parts:?}");
        std::fs::write(dir.join(".entry.lock"), b"").unwrap();

        assert_eq!(remove_parts(&dir.join("entry")).unwrap(), 1);
        assert_eq!(remove_parts(&dir.join("download")).unwrap(), 1);
        assert_eq!(names(&dir), [".entry.lock", "ready"]);
        PartDir::beside(&dir.join("entry")).unwrap().keep().unwrap();
        assert!(dir.join("entry").is_dir());
    }

    #[tokio::test]
    async fn a_lock_keeps_a_second_holder_waiting_until_it_is_dropped() {
        let dir = dir("lock");
        let path = dir.join(".entry.lock");
        let first = Lock::acquire(&path).await.unwrap();
        // A second open of the file is a second holder, in this process as
        // in another.
        let other = std::fs::File::open(&path).unwrap();
        assert!(matches!(
            other.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = tokio::spawn({
            let path = path.clone();
            async move {
                let second = Lock::acquire(&path).await.unwrap();
                tx.send(()).unwrap();
                second
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(rx.try_recv().is_err(), "the second took a held lock");
        drop(first);
        let second = waiter.await.unwrap();
        assert!(rx.try_recv().is_ok());
        drop(second);
        other.try_lock().unwrap();
        sync_dir(&dir).unwrap();
        assert!(sync_dir(&dir.join("missing")).is_err());
    }

    #[test]
    fn a_part_that_cannot_be_kept_is_removed() {
        let dir = dir("refused");
        // A directory with something in it is no target for a file.
        let target = dir.join("taken");
        std::fs::create_dir_all(target.join("inside")).unwrap();
        let err = write_whole(&target, b"bytes").unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::NotFound, "{err}");
        assert_eq!(names(&dir), ["taken"]);
    }
}
