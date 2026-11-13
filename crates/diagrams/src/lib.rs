//! Keeps kai's diagrams true to the code.
//!
//! A diagram in a Markdown file is a ```` ```mermaid ```` block. Its first
//! lines say how it stays true ([`header`]): a test writes it
//! (`generated`), a test compares it with the Rust it shows (`checked`), or
//! a test notices when the code it was drawn from changes (`drawn`, with a
//! stamp). [`check`] runs those rules over the repository's Markdown;
//! [`graph`] writes the crate graph; [`stubs`] goes the other way and
//! prints Rust stubs from a planned class diagram.
//!
//! Everything here reads text: Markdown, Mermaid and Rust source. Nothing
//! builds a crate of kai's, calls a model or needs the network.
//!
//! The parts, in the order they depend on each other:
//!
//! - [`markdown`] finds the Mermaid blocks of a Markdown file;
//! - [`mermaid`] parses the subset of Mermaid kai uses, refusing any line
//!   outside it, so that a check cannot pass by reading nothing;
//! - [`model`] is what a class says, from the diagram's side and from the
//!   code's, and the comparison between the two;
//! - [`rust`] reads a crate's source with syn into that model;
//! - [`header`], [`check`], [`graph`] and [`stubs`] use them.

pub mod check;
pub mod graph;
pub mod header;
pub mod markdown;
pub mod mermaid;
pub mod model;
pub mod rust;
pub mod stubs;

use std::fmt;

/// One thing a diagram says that does not hold, at the line that says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The file, as given to the check (relative to the repository root).
    pub file: String,
    /// The line in that file, from 1.
    pub line: usize,
    /// What does not hold, and where the code says otherwise.
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}

/// What every failing check says after its findings.
pub const BOTH_WAYS: &str = "If the code changed on purpose, change the diagram; if not, the change in the code is the bug.";
