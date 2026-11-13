//! What kai does, as a library. It knows nothing of the command line: `cli`
//! calls it and prints. It logs through `tracing` and never decides where
//! the lines go. For now it knows the greeting, [`qualify`]: whether a
//! ticket may be used, judged from what GitHub says about it, and
//! [`deps`]: a ticket's repository and its dependencies, fetched and built
//! in microVMs into two layers. Agent programs and their layers belong to
//! the separate `agent` crate.

pub mod deps;
pub mod qualify;

/// The line kai greets with.
pub fn greeting() -> &'static str {
    tracing::debug!("the greeting was asked for");
    "hello kai"
}
