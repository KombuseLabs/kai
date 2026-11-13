//! `kai agent`: an agent in a microVM. For now its pinned program as a
//! read-only layer, in kai's VM cache.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::{Args, Subcommand};
use tokio::runtime::Runtime;

use super::{Context, Failure};
use crate::settings;

#[derive(Args)]
pub(crate) struct AgentArgs {
    /// kai's VM cache, where the agent's layer is kept; without it, vm.cache in kai.toml, else ~/.cache/kai/vm
    #[arg(long, global = true, env = "KAI_VM_CACHE", value_name = "DIR")]
    cache: Option<PathBuf>,

    #[command(subcommand)]
    command: AgentCommand,
}

#[derive(Subcommand)]
enum AgentCommand {
    /// Downloads the pinned Claude Code, checks it against its SHA-256, packs it as a read-only layer, every file root's, and prints the layer's path; a layer in the cache that checks out is used as it is, one that does not is moved aside and made again
    Fetch,
}

pub(crate) fn dispatch(args: &AgentArgs, context: &Context<'_>) -> Result<(), Failure> {
    let cache =
        settings::vm_cache(args.cache.as_deref(), context.config).map_err(Failure::Usage)?;
    match &args.command {
        AgentCommand::Fetch => fetch(&cache, context.runtime),
    }
}

/// Puts Claude Code into the cache as a layer and says what it did.
fn fetch(cache: &Path, runtime: &Runtime) -> Result<(), Failure> {
    let agent = &::agent::CLAUDE;
    let fetched = runtime
        .block_on(::agent::layer::fetch(agent, cache))
        .with_context(|| format!("cannot fetch {}", agent.binary.name))?;
    if fetched.removed > 0 {
        println!(
            "{}: removed {} left over by a fetch that was stopped",
            agent.binary.name, fetched.removed
        );
    }
    if let Some((aside, why)) = &fetched.quarantined {
        println!(
            "{}: the layer in the cache did not check out ({why}); moved to {}, which can go once no VM uses it",
            agent.binary.name,
            aside.display()
        );
    }
    let what = match (fetched.reused, fetched.downloaded) {
        (true, _) => "already there, checked",
        (false, true) => "downloaded, checked, packed",
        (false, false) => "packed from the checked download in the cache",
    };
    println!(
        "{}: {what}; layer {}",
        agent.binary.name,
        fetched.file.display()
    );
    Ok(())
}
