//! Files put in place whole. A file is written, or made by a program, as
//! a [`Part`] beside its target and renamed to the target only once it is
//! complete: a reader finds the old file or the new one, never half of
//! one, and a failure leaves nothing behind.
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

impl Part {
    /// The part for `target`. Nothing is made yet.
    pub fn beside(target: &Path) -> Part {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = target.file_name().map_or_else(
            || "file".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        Part {
            path: target.with_file_name(format!(".{name}.{}-{n}.part", std::process::id())),
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
