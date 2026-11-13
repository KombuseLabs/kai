//! The commands. Each module holds a group's arguments (clap) and what the
//! group does; this one only hands a command to its group. A new group is a
//! new module and a line in [`Command`] and [`run`].
//!
//! The crates kai is made of are named with a leading `::`, since some of
//! these modules share their names (`github`, `store`, `vm`).

mod agent;
mod github;
mod proxy;
mod store;
mod study;
mod ticket;
mod vm;

use std::path::PathBuf;

use clap::Subcommand;
use helpers::config::Loaded;

use crate::settings::Config;

/// kai's command groups.
#[derive(Subcommand)]
pub(crate) enum Command {
    /// Hold a private connection handoff for another terminal; started by tools/start-proxy
    Proxy,
    /// Run a named comparison from the configured runs directory
    Run(agent::NamedRunArgs),
    /// Reads tickets from GitHub
    #[command(subcommand)]
    Github(github::GithubCommand),
    /// The tickets in the store
    #[command(subcommand)]
    Ticket(ticket::TicketCommand),
    /// The store, kai's cache of the tickets it fetched
    #[command(subcommand)]
    Store(store::StoreCommand),
    /// Studies: comparing variants of an agent on tickets
    #[command(subcommand)]
    Study(study::StudyCommand),
    /// microVMs: what running one needs, the pinned Firecracker and guest kernel, and one command in a fresh VM
    Vm(vm::VmArgs),
    /// Pinned agent runtimes and one-shot or interactive microVM sessions
    Agent(Box<agent::AgentArgs>),
}

/// What every command may need: the global options, resolved against the
/// settings file, and the async runtime.
pub(crate) struct Context<'a> {
    /// The store's file.
    pub(crate) store: PathBuf,
    /// The settings file, if one was read.
    pub(crate) config: Option<&'a Loaded<Config>>,
    /// The runtime `main` started, on which a command runs what waits on
    /// the network or on a VM.
    pub(crate) runtime: &'a tokio::runtime::Runtime,
}

/// Why a command did not succeed, and so the exit status.
pub(crate) enum Failure {
    /// The command line or the settings lack what the command needs: 2.
    Usage(anyhow::Error),
    /// The command failed while it ran: 1.
    Run(anyhow::Error),
    /// The exit status of a command kai ran for the person, passed on as
    /// it is; what to say was said already.
    Status(u8),
}

impl From<anyhow::Error> for Failure {
    fn from(err: anyhow::Error) -> Failure {
        Failure::Run(err)
    }
}

/// Runs `command`; without one, kai greets.
pub(crate) fn run(command: Option<&Command>, context: &Context<'_>) -> Result<(), Failure> {
    match command {
        Some(Command::Proxy) => proxy::publish(context),
        Some(Command::Run(args)) => agent::run_named(args, context),
        None => {
            println!("{}", ::engine::greeting());
            Ok(())
        }
        Some(Command::Github(command)) => github::run(command, context),
        Some(Command::Ticket(command)) => ticket::run(command, context),
        Some(Command::Store(command)) => store::run(command, context),
        Some(Command::Study(command)) => study::run(command, context),
        Some(Command::Vm(args)) => vm::dispatch(args, context),
        Some(Command::Agent(args)) => agent::dispatch(args, context),
    }
}
