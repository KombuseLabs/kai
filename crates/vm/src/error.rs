//! The crate's error type.

use std::io;
use std::path::PathBuf;

/// What went wrong on the host. A download that fails removes whatever it
/// wrote.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The URL is plain HTTP to another machine, or not a URL kai takes.
    #[error("`{0}` is not an https:// URL (plain http:// is taken only for this machine)")]
    Url(String),
    /// The request did not get an answer: the network, TLS, a timeout.
    #[error("cannot download {url}: {message}")]
    Download {
        /// What was asked for.
        url: String,
        /// Why not, with every cause.
        message: String,
    },
    /// The server answered with an HTTP error.
    #[error("{url} answered {status}")]
    Status {
        /// What was asked for.
        url: String,
        /// The HTTP status.
        status: u16,
    },
    /// The bytes are not the pinned ones.
    #[error("{what} has SHA-256 {actual}, not the pinned {expected}; nothing was kept")]
    Checksum {
        /// The download's URL, or the member of an archive.
        what: String,
        /// The pin.
        expected: &'static str,
        /// What the bytes hash to.
        actual: String,
    },
    /// `tar` could not take the member out of the archive.
    #[error("cannot unpack {member}: {message}")]
    Unpack {
        /// The member, as the pin names it.
        member: String,
        /// `tar`'s words, or why it did not start.
        message: String,
    },
    /// Reading or writing a file of the cache failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The error.
        source: io::Error,
    },
    /// kai was built without its guest init; the text says why.
    #[error("this kai was built without its guest init: {0}")]
    NoInit(String),
    /// An artefact a run needs is missing or not the pinned file.
    #[error("{name}: {problem}")]
    Artefact {
        /// The artefact, as the pins name it.
        name: &'static str,
        /// What is wrong, and what fixes it.
        problem: String,
    },
    /// A recipe kai does not have, or one that breaks the recipe rules.
    #[error("{0}")]
    Recipe(String),
    /// The image of a recipe is not built yet.
    #[error("the image of the recipe `{0}` is not built; `kai vm image build {0}` builds it")]
    NotBuilt(String),
    /// Docker, while an image was being made.
    #[error("docker {step}: {message}")]
    Docker {
        /// What kai asked Docker to do.
        step: &'static str,
        /// Docker's words, or why it did not start.
        message: String,
    },
    /// A program kai runs on the host failed.
    #[error("{program}: {message}")]
    Tool {
        /// The program.
        program: &'static str,
        /// Its words, or why it did not start.
        message: String,
    },
    /// A new image did not run `sh -c true` in a VM, so it was not kept.
    #[error("the new image did not pass its probe run ({outcome}); see {}", run_dir.display())]
    Probe {
        /// How the probe run ended.
        outcome: String,
        /// Its run directory, kept.
        run_dir: PathBuf,
    },
    /// The run was refused before anything was made: what it was given does
    /// not fit.
    #[error("{0}")]
    Refused(String),
}
