//! What kai does, as a library. It knows nothing of the command line: `cli`
//! calls it and prints. It logs through `tracing` and never decides where
//! the lines go. For now it only knows the greeting.

/// The line kai greets with.
pub fn greeting() -> &'static str {
    tracing::debug!("the greeting was asked for");
    "hello kai"
}
