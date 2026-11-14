//! `kai github`: reads tickets from GitHub, never writes there.

use anyhow::Context as _;
use clap::Subcommand;
use types::RepoName;

use super::{Context, Failure};
use crate::settings;

#[derive(Subcommand)]
pub(crate) enum GithubCommand {
    /// Fetches the board's issues changed since the last sync, and lists its files, into the store; writes nothing to GitHub.
    ///
    /// The mirror service supplies its own GitHub credentials.
    Sync {
        /// The repository whose issues are the tickets; without it, github.board in kai.toml
        #[arg(long, env = "KAI_GITHUB_BOARD", value_name = "OWNER/NAME")]
        board: Option<RepoName>,

        /// Fetch every issue, not only those changed since the last sync, and remove those GitHub no longer returns
        #[arg(long)]
        full: bool,

        /// Also ask GitHub what access each account that put a label on has to the board now, one request per account
        #[arg(long)]
        actor_permissions: bool,
    },
}

pub(crate) fn run(command: &GithubCommand, context: &Context<'_>) -> Result<(), Failure> {
    match command {
        GithubCommand::Sync {
            board,
            full,
            actor_permissions,
        } => {
            let board = settings::board(board.as_ref(), context.config).map_err(Failure::Usage)?;
            sync(context, &board, *full, *actor_permissions)
        }
    }
}

/// Syncs through the service without opening a database or sending a GitHub token.
fn sync(
    context: &Context<'_>,
    board: &RepoName,
    full: bool,
    actor_permissions: bool,
) -> Result<(), Failure> {
    let store = super::store::open(&context.store)?;
    let result = context
        .runtime
        .block_on(store.sync(board, full, actor_permissions))
        .with_context(|| format!("cannot fetch the issues of {board}"))?;
    let removed = result
        .removed
        .map(|n| format!(", {n} removed that GitHub no longer returns"))
        .unwrap_or_default();
    println!(
        "{board}: {} issues fetched{removed}, {} kept by {}",
        result.fetched, result.kept, context.store
    );
    if let Some(why) = result.board_files_unknown {
        println!("{board}: files not listed, the listing of before is kept: {why}");
    }
    Ok(())
}
