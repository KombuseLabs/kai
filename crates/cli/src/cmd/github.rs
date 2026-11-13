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
            let api = settings::api(context.config);
            sync(context, &board, api, *full, *actor_permissions)
        }
    }
}

/// Fetches `board`'s issues from the API at `api` into the store: all of
/// them with `full`, else those changed since the last sync. With
/// `actor_permissions`, then the access of every account that put a label
/// on an issue the store holds.
fn sync(
    context: &Context<'_>,
    board: &RepoName,
    api: &str,
    full: bool,
    actor_permissions: bool,
) -> Result<(), Failure> {
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
    if actor_permissions {
        let actors = store.actors(board).context("cannot read the store")?;
        let mut answers = Vec::with_capacity(actors.len());
        for login in actors {
            let access = context
                .runtime
                .block_on(client.access(board, &login))
                .with_context(|| format!("cannot ask GitHub for the access of {login}"))?;
            answers.push((login, access));
        }
        store
            .record_access(board, &answers)
            .context("cannot write the store")?;
        let unknown = answers
            .iter()
            .filter(|(_, access)| matches!(access, ::types::Access::Unknown(_)))
            .count();
        println!(
            "{board}: access of {} label actors asked, {unknown} not told",
            answers.len()
        );
    }
    Ok(())
}
