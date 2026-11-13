//! `kai github`: reads tickets from GitHub, never writes there.

use anyhow::Context as _;
use clap::Subcommand;
use types::RepoName;

use super::{Context, Failure};
use crate::settings;

/// The variable that holds the GitHub token.
const TOKEN_VARIABLE: &str = "KAI_GITHUB_TOKEN";

#[derive(Subcommand)]
pub(crate) enum GithubCommand {
    /// Fetches the board's issues changed since the last sync into the store; writes nothing to GitHub.
    ///
    /// The token is KAI_GITHUB_TOKEN, else what `gh auth token` prints.
    Sync {
        /// The repository whose issues are the tickets; without it, github.board in kai.toml
        #[arg(long, env = "KAI_GITHUB_BOARD", value_name = "OWNER/NAME")]
        board: Option<RepoName>,

        /// Fetch every issue, not only those changed since the last sync, and remove those GitHub no longer returns
        #[arg(long)]
        full: bool,
    },
}

pub(crate) fn run(command: &GithubCommand, context: &Context<'_>) -> Result<(), Failure> {
    match command {
        GithubCommand::Sync { board, full } => {
            let board = settings::board(board.as_ref(), context.config).map_err(Failure::Usage)?;
            sync(context, &board, settings::api(context.config), *full)
        }
    }
}

/// Fetches `board`'s issues from the API at `api` into the store: all of
/// them with `full`, else those changed since the last sync.
fn sync(context: &Context<'_>, board: &RepoName, api: &str, full: bool) -> Result<(), Failure> {
    let store_path = &context.store;
    let mut store = super::store::open(store_path)?;
    let token = match std::env::var(TOKEN_VARIABLE) {
        Ok(token) if !token.is_empty() => ::github::SecretString::from(token),
        _ => ::github::token_from_gh()
            .with_context(|| format!("{TOKEN_VARIABLE} is not set"))
            .map_err(Failure::Usage)?,
    };
    let client = ::github::Client::new(token, api).map_err(|err| Failure::Usage(err.into()))?;
    let since = if full {
        None
    } else {
        store.cursor(board).context("cannot read the store")?
    };
    let fetched = context
        .runtime
        .block_on(client.fetch_board(board, since.as_deref()))
        .with_context(|| format!("cannot fetch the issues of {board}"))?;
    let removed = store
        .record_sync(board, &fetched.issues, fetched.since.as_deref(), full)
        .context("cannot write the store")?;
    tracing::info!(%board, fetched = fetched.issues.len(), removed, since = ?since, "synced");
    let kept = store
        .boards()
        .context("cannot read the store")?
        .iter()
        .find(|synced| &synced.board == board)
        .map_or(0, |synced| synced.issues);
    // Only a full sync can tell what GitHub no longer has.
    let removed = if full {
        format!(", {removed} removed that GitHub no longer returns")
    } else {
        String::new()
    };
    println!(
        "{board}: {} issues fetched{removed}, {kept} kept in {}",
        fetched.issues.len(),
        store_path.display()
    );
    Ok(())
}
