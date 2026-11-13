//! The commands. Each module holds a group's arguments (clap) and what the
//! group does; this one only hands a command to its group. A new group is a
//! new module and a line in [`Command`] and [`run`].
//!
//! The crates kai is made of are named with a leading `::`, since some of
//! these modules share their names (`github`, `store`).

mod github;
mod store;
mod ticket;

use std::path::PathBuf;

use clap::Subcommand;
use helpers::config::Loaded;

use crate::settings::Config;

/// kai's command groups.
#[derive(Subcommand)]
pub(crate) enum Command {
    /// Reads tickets from GitHub
    #[command(subcommand)]
    Github(github::GithubCommand),
    /// The tickets in the store
    #[command(subcommand)]
    Ticket(ticket::TicketCommand),
    /// The store, kai's cache of the tickets it fetched
    #[command(subcommand)]
    Store(store::StoreCommand),
}

/// What every command may need: the global options, resolved against the
/// settings file.
pub(crate) struct Context<'a> {
    /// The store's file.
    pub(crate) store: PathBuf,
    /// The settings file, if one was read.
    pub(crate) config: Option<&'a Loaded<Config>>,
}

/// Why a command did not succeed, and so the exit status.
pub(crate) enum Failure {
    /// The command line or the settings lack what the command needs: 2.
    Usage(anyhow::Error),
    /// The command failed while it ran: 1.
    Run(anyhow::Error),
}

impl From<anyhow::Error> for Failure {
    fn from(err: anyhow::Error) -> Failure {
        Failure::Run(err)
    }
}

/// Runs `command`; without one, kai greets.
pub(crate) fn run(command: Option<&Command>, context: &Context<'_>) -> Result<(), Failure> {
    match command {
        None => {
            println!("{}", ::engine::greeting());
            Ok(())
        }
        Some(Command::Github(command)) => github::run(command, context),
        Some(Command::Ticket(command)) => ticket::run(command, context),
        Some(Command::Store(command)) => store::run(command, context),
    }
}
