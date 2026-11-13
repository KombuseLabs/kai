//! A study: variants of a coding agent compared on tickets, by the rule of
//! the study spec. What is here is pure computation, with no VM and no
//! model:
//!
//! - [`plan`]: SplitMix64 and the counterbalanced order of the trials;
//! - [`stats`]: the paired statistics, ported from old kai;
//! - [`verdict`]: one ghost against the shadow, `better`, `worse` or
//!   `not-distinguishable`, with the sensitivity analysis of `limit`s;
//! - [`estimate`]: what a study of a given size can see, before it is run.
//!
//! The data these use is in `types::study`.

pub mod estimate;
pub mod plan;
pub mod stats;
pub mod verdict;
