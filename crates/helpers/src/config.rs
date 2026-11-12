//! Settings files: a program names the file it looks for and the type the
//! file is read into, and gets back what the file says or why it could not
//! be read.
//!
//! - **TOML, into any type that implements `serde::Deserialize`.** A type
//!   should say `#[serde(deny_unknown_fields)]`, so that a misspelt key is
//!   an error and never a silent default.
//! - **Found from where the program runs.** [`find`] looks in a directory,
//!   then in each one above it, and takes the first file of that name, as
//!   git and cargo find theirs.
//! - **A path in the file is relative to the file**, not to the directory
//!   the program runs in: [`Loaded::resolve`].
//! - **An error names the file**, and for a malformed one the line and the
//!   column.
//!
//! What the settings mean, and which wins over which (an option, an
//! environment variable, the file, a default), is the program's to decide;
//! this module only finds and reads.
//!
//! ```
//! use helpers::config;
//! use serde::Deserialize;
//! use std::path::PathBuf;
//!
//! #[derive(Deserialize)]
//! #[serde(deny_unknown_fields)]
//! struct Settings {
//!     seed: Option<u64>,
//!     log: Option<PathBuf>,
//! }
//!
//! let dir = std::env::temp_dir().join("helpers-config-doc");
//! std::fs::create_dir_all(dir.join("sub"))?;
//! std::fs::write(dir.join("example.toml"), "seed = 7\nlog = \"logs/run.jsonl\"\n")?;
//!
//! // Found from a directory below the one that holds it.
//! let path = config::find(&dir.join("sub"), "example.toml").expect("one directory up");
//! let settings: config::Loaded<Settings> = config::load(&path)?;
//! assert_eq!(settings.value.seed, Some(7));
//! let log = settings.value.log.as_deref().map(|log| settings.resolve(log));
//! assert_eq!(log, Some(dir.join("logs/run.jsonl")));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fs;
use std::io;
use std::path::{self, Path, PathBuf};

use serde::de::DeserializeOwned;

/// The first file called `name` in `start` or in a directory above it, or
/// `None`. A relative `start` is taken from the current directory. A
/// directory of that name is passed over; a file that exists but cannot be
/// read is returned, so that [`load`] says why.
pub fn find(start: &Path, name: &str) -> Option<PathBuf> {
    let start = path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
    start
        .ancestors()
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Reads the TOML file at `path` into a `T`.
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<Loaded<T>, Error> {
    let text = fs::read_to_string(path).map_err(|source| Error::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value = toml::from_str(&text).map_err(|source| Error::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(Loaded {
        value,
        path: path.to_path_buf(),
    })
}

/// Settings read from a file, and the file they were read from.
#[derive(Debug)]
pub struct Loaded<T> {
    /// What the file says.
    pub value: T,
    /// The file, as it was named or found.
    pub path: PathBuf,
}

impl<T> Loaded<T> {
    /// A path the file names, taken from the file's directory instead of
    /// the current one. An absolute path stays as it is.
    pub fn resolve(&self, path: &Path) -> PathBuf {
        match self.path.parent() {
            Some(dir) => dir.join(path),
            None => path.to_path_buf(),
        }
    }
}

/// Why a settings file could not be read. It names the file; its
/// [`source`](std::error::Error::source) is the cause.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file is missing or cannot be opened, or it is not UTF-8.
    #[error("cannot read {}", .path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// Why it cannot be read.
        source: io::Error,
    },
    /// The file is not TOML, or does not fit the type it is read into.
    #[error("{} does not hold valid settings", .path.display())]
    Parse {
        /// The file.
        path: PathBuf,
        /// What does not fit, with the line and the column.
        source: toml::de::Error,
    },
}
