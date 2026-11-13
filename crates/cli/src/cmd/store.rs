//! `kai store`: the store itself, kai's mirror of the issues it fetched.

use std::path::Path;

use anyhow::Context as _;
use clap::Subcommand;

use super::{Context, Failure};

#[derive(Subcommand)]
pub(crate) enum StoreCommand {
    /// Empties the store: every issue and what it knew of each board's syncs; the next sync fetches everything again
    Reset,
    /// Says for each board in the store when it was last synced, when last in full, and how many issues it holds
    Status,
}

pub(crate) fn run(command: &StoreCommand, context: &Context<'_>) -> Result<(), Failure> {
    match command {
        StoreCommand::Reset => reset(&context.store),
        StoreCommand::Status => status(&context.store),
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
    println!("{}: emptied, {removed} issues removed", path.display());
    Ok(())
}

/// One line per board the store at `path` has a sync of. Where there is
/// no store, nothing is made.
fn status(path: &Path) -> Result<(), Failure> {
    if !path.exists() {
        println!(
            "{}: no store here; `kai github sync` makes one",
            path.display()
        );
        return Ok(());
    }
    let boards = open(path)?.boards().context("cannot read the store")?;
    if boards.is_empty() {
        println!("{}: no board synced yet", path.display());
    }
    for synced in boards {
        let full = synced.last_full_sync.unwrap_or_else(|| "never".to_owned());
        println!(
            "{}: {} issues, last synced {}, last in full {full}",
            synced.board, synced.issues, synced.last_sync
        );
    }
    Ok(())
}
