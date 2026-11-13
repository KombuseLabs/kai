//! kai's domain types: what a ticket is, and what qualifying it found.
//!
//! Data only. The rules that judge a ticket are `engine`'s; fetching,
//! keeping and printing tickets belong to the crates that do I/O. This
//! crate depends on nothing of kai's, so that every other crate can depend
//! on it.
//!
//! A value nobody recorded stays `None`; it is never filled in with a
//! default.

mod qualification;
mod repo;
pub mod study;
mod ticket;

pub use qualification::{Finding, Outcome, Qualification, Rule, Verdict};
pub use repo::{IssueId, ParseRepoNameError, RepoName};
pub use ticket::{Form, IssueState, LabelEvent, Ticket};
