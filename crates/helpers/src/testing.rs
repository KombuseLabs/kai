//! What the tests of every crate share; compiled only for tests (the
//! feature `test-support`, which the other crates turn on in their
//! `[dev-dependencies]`).
//!
//! [`TestDir`] is a test's own directory. It lives under one parent,
//! `kai-tests` in the system's temporary directory, and is removed when it
//! is dropped, so a test leaves nothing behind when it returns or panics. A
//! test process that is killed cannot remove its directories; the next test
//! process removes them when it makes its first one.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};

/// The name of the parent of every [`TestDir`], in the system's temporary
/// directory.
pub const PARENT: &str = "kai-tests";

/// An empty directory of a test's own, removed with everything in it when it
/// is dropped. It reads as its path: `dir.join("file")`, `&dir` where a
/// `&Path` is wanted.
#[derive(Debug)]
pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    /// A fresh, empty directory for `name`, unique within the process: its
    /// name is the process's id, `name` and a number.
    ///
    /// # Panics
    ///
    /// If the directory cannot be made: a test cannot go on without it.
    pub fn new(name: &str) -> TestDir {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::temp_dir().join(PARENT);
        static SWEPT: Once = Once::new();
        SWEPT.call_once(|| sweep(&parent));
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("{}-{name}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap_or_else(|e| panic!("making {}: {e}", path.display()));
        TestDir { path }
    }

    /// The directory's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl std::ops::Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<OsStr> for TestDir {
    fn as_ref(&self) -> &OsStr {
        self.path.as_os_str()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Removes what test processes that are gone left in `parent`: every entry
/// whose name does not start with the id of a running process. A running
/// process's directories, such as those of a test running beside this one,
/// are left alone.
fn sweep(parent: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid = name
            .to_string_lossy()
            .split('-')
            .next()
            .unwrap_or("")
            .to_owned();
        let running = !pid.is_empty()
            && pid.bytes().all(|b| b.is_ascii_digit())
            && Path::new("/proc").join(&pid).exists();
        if !running {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dir_is_empty_unique_and_gone_when_dropped() {
        let a = TestDir::new("unit");
        let b = TestDir::new("unit");
        assert_ne!(a.path(), b.path());
        assert!(a.is_dir());
        assert_eq!(std::fs::read_dir(&a).unwrap().count(), 0);
        std::fs::write(a.join("file"), b"x").unwrap();
        let path = a.to_path_buf();
        drop(a);
        assert!(!path.exists());
    }

    #[test]
    fn a_dir_is_gone_after_a_panic() {
        let path = std::panic::catch_unwind(|| {
            let dir = TestDir::new("panics");
            std::fs::write(dir.join("file"), b"x").unwrap();
            let path = dir.to_path_buf();
            std::panic::panic_any(path);
        })
        .unwrap_err()
        .downcast::<PathBuf>()
        .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn what_a_gone_process_left_is_swept() {
        let parent = TestDir::new("sweep");
        let gone = parent.join("4294967294-left-0");
        let mine = parent.join(format!("{}-mine-0", std::process::id()));
        for dir in [&gone, &mine] {
            std::fs::create_dir_all(dir.join("inner")).unwrap();
        }
        sweep(&parent);
        assert!(!gone.exists());
        assert!(mine.exists());
    }
}
