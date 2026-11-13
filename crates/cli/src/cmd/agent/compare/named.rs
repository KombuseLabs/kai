//! Named comparisons are plain JSON files, not a separate execution path.
use super::{CompareArgs, Context, Failure, ProfileArgs, dispatch};
use crate::settings;
use anyhow::Context as _;
use clap::Args;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Args)]
pub(crate) struct NamedRunArgs {
    /// JSON filename without .json, in runs.directory from kai.toml
    name: String,
    /// Override the VM cache
    #[arg(long, env = "KAI_VM_CACHE", value_name = "DIR")]
    cache: Option<PathBuf>,
    /// New output directory; by default each call creates one in the VM cache
    #[arg(long, value_name = "DIR")]
    output: Option<PathBuf>,
    /// Use an explicitly supplied local fake API
    #[arg(long)]
    mock: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    model: String,
    prompt: String,
    extra_prompt: Option<String>,
    extra_prompt_file: Option<PathBuf>,
    addon: Option<String>,
    tree: Option<PathBuf>,
    max_turns: Option<u32>,
    #[serde(default)]
    sequential: bool,
}

fn load(directory: &Path, name: &str) -> anyhow::Result<(Definition, PathBuf)> {
    anyhow::ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c)),
        "run name must contain only letters, digits, - or _ (without .json)"
    );
    let file = directory.join(format!("{name}.json"));
    let bytes =
        std::fs::read(&file).with_context(|| format!("cannot read run {}", file.display()))?;
    let definition: Definition = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid run {}", file.display()))?;
    anyhow::ensure!(
        definition.extra_prompt.is_some() != definition.extra_prompt_file.is_some(),
        "{} must set exactly one of extra_prompt and extra_prompt_file",
        file.display()
    );
    anyhow::ensure!(
        !definition.model.trim().is_empty(),
        "run model must not be empty"
    );
    if let Some(turns) = definition.max_turns {
        anyhow::ensure!(
            (1..=::agent::session::MAX_TURNS).contains(&turns),
            "max_turns must be 1 to 10000"
        );
    }
    Ok((definition, file))
}

pub(crate) fn run_named(args: &NamedRunArgs, context: &Context<'_>) -> Result<(), Failure> {
    let directory = settings::runs_directory(context.config);
    let (definition, file) = load(&directory, &args.name).map_err(Failure::Usage)?;
    let parent = file.parent().expect("run file has its directory");
    let cache =
        settings::vm_cache(args.cache.as_deref(), context.config).map_err(Failure::Usage)?;
    let compare = CompareArgs {
        profile: ProfileArgs {
            model: definition.model,
            image: "rust".into(),
            tree: definition.tree.map(|path| parent.join(path)),
            layers: vec![],
            cpus: 2,
            memory: 4096,
            timeout: 600,
            tools: ::agent::session::TOOLS.join(","),
            max_turns: definition.max_turns,
            mock: args.mock,
        },
        extra_prompt: definition.extra_prompt,
        extra_prompt_file: definition.extra_prompt_file.map(|path| parent.join(path)),
        addon: definition.addon,
        output: Some(
            args.output
                .clone()
                .unwrap_or_else(|| ::vm::new_run_dir(&cache, Some(&args.name))),
        ),
        sequential: definition.sequential,
        prompt: definition.prompt,
    };
    dispatch(&compare, &cache, context)
}
