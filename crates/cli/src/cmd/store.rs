//! `kai store`: the store itself, kai's cache of the tickets it fetched.

use std::path::Path;

use anyhow::Context as _;
use clap::Subcommand;

use super::{Context, Failure};

#[derive(Subcommand)]
pub(crate) enum StoreCommand {
    /// Empties the store: every ticket and where each board's last sync stopped; the next sync fetches everything again
    Reset,
}

pub(crate) fn run(command: &StoreCommand, context: &Context<'_>) -> Result<(), Failure> {
    match command {
        StoreCommand::Reset => reset(&context.store),
    }
}

/// Opens the store at `path`, as every command does.
pub(super) fn open(path: &Path) -> anyhow::Result<::store::Store> {
    ::store::Store::open(path).with_context(|| format!("cannot open the store {}", path.display()))
}

/// Empties the store at `path`. Where there is none, nothing is made.
fn reset(path: &Path) -> Result<(), Failure> {
    if !path.exists() {
        println!("{}: no store here; nothing to empty", path.display());
        return Ok(());
    }
    let removed = open(path)?.clear().context("cannot empty the store")?;
    println!("{}: emptied, {removed} tickets removed", path.display());
    Ok(())
}
