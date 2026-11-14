//! Commands for the external ticket mirror.
use super::{Context, Failure};
use anyhow::Context as _;
use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum StoreCommand {
    /// Empty the service's cache; its next sync fetches everything again
    Reset,
    /// Show the mirror's boards, sync times and observed actor permissions
    Status,
}

pub(super) fn open(url: &str) -> anyhow::Result<::store::Store> {
    ::store::Store::new(url).with_context(|| format!("cannot connect to the store {url}"))
}

pub(crate) fn run(command: &StoreCommand, context: &Context<'_>) -> Result<(), Failure> {
    let store = open(&context.store)?;
    match command {
        StoreCommand::Reset => {
            let removed = context
                .runtime
                .block_on(store.clear())
                .context("cannot empty the store")?;
            println!("{}: emptied, {removed} issues removed", context.store);
        }
        StoreCommand::Status => {
            let snapshot = context
                .runtime
                .block_on(store.snapshot())
                .context("cannot read the store")?;
            if snapshot.boards.is_empty() {
                println!(
                    "{}: no board synced yet; `kai github sync` fills it",
                    context.store
                );
            }
            for board in snapshot.boards {
                let full = board.last_full_sync.unwrap_or_else(|| "never".into());
                println!(
                    "{}: {} issues, last synced {}, last in full {full}",
                    board.repo, board.issues, board.last_sync
                );
                for actor in board.access {
                    let access = match actor.access {
                        ::types::Access::Known { permission, role } => match role {
                            Some(role) => format!("{permission} (role {role})"),
                            None => permission,
                        },
                        ::types::Access::Unknown(why) => format!("not told: {why}"),
                    };
                    println!("  {}: {access}, asked {}", actor.login, actor.observed_at);
                }
            }
        }
    }
    Ok(())
}
