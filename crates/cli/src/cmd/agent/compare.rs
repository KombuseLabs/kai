//! Two independent one-shot sessions, with their results kept apart.
use super::{Context, Failure, ProfileArgs, execute};
use anyhow::Context as _;
use clap::Args;
use helpers::fs::write_whole;
use std::path::{Path, PathBuf};

pub(super) mod named;

#[derive(Args)]
pub(super) struct CompareArgs {
    #[command(flatten)]
    profile: ProfileArgs,
    /// Additional instructions, placed before the task in the second run only
    #[arg(
        long,
        value_name = "TEXT",
        required_unless_present = "extra_prompt_file",
        conflicts_with = "extra_prompt_file"
    )]
    extra_prompt: Option<String>,
    /// Read the additional instructions from this UTF-8 file
    #[arg(long, value_name = "FILE")]
    extra_prompt_file: Option<PathBuf>,
    /// Load this pinned add-on only in the second run; the extra prompt must be its activation slash command
    #[arg(long, value_name = "NAME", value_parser = clap::builder::PossibleValuesParser::new(::agent::addon::ADDONS.iter().map(|addon| addon.name)))]
    addon: Option<String>,
    /// New result directory, outside --tree; defaults to a new directory in the VM cache
    #[arg(long, value_name = "DIR")]
    output: Option<PathBuf>,
    /// Run one VM at a time instead of both in parallel
    #[arg(long)]
    sequential: bool,
    /// The same task for both runs, as one argument; never interpreted by a shell
    #[arg(last = true, required = true)]
    prompt: String,
}

pub(super) fn dispatch(
    args: &CompareArgs,
    cache: &Path,
    context: &Context<'_>,
) -> Result<(), Failure> {
    let extra = match (&args.extra_prompt, &args.extra_prompt_file) {
        (Some(text), _) => text.clone(),
        (_, Some(path)) => std::fs::read_to_string(path)
            .with_context(|| format!("cannot read extra prompt {}", path.display()))
            .map_err(Failure::Usage)?,
        _ => unreachable!("clap requires an extra prompt"),
    };
    let mut with_prompt = prompts(&args.prompt, &extra).map_err(Failure::Usage)?;
    let addon = args
        .addon
        .as_deref()
        .map(|name| {
            let addon = ::agent::addon::named(name)
                .ok_or_else(|| Failure::Usage(anyhow::anyhow!("unknown add-on: {name}")))?;
            if ::agent::session::command(&extra).is_none() {
                return Err(Failure::Usage(anyhow::anyhow!(
                    "an add-on's extra prompt must be an activation slash command"
                )));
            }
            Ok(addon)
        })
        .transpose()?;
    let output = std::path::absolute(
        args.output
            .clone()
            .unwrap_or_else(|| ::vm::new_run_dir(cache, Some("compare"))),
    )
    .map_err(|e| Failure::Usage(e.into()))?;
    let mut without = args.profile.options(output.join("without/run"))?;
    let mut with = args.profile.options(output.join("with/run"))?;
    let passed = context.runtime.block_on(async {
        if let Some(addon) = addon {
            with.addons
                .push(::agent::addon::cached(addon, cache).await?);
            with.activation = Some(extra.clone());
            // Session::send adds the activation exactly once.
            with_prompt = args.prompt.clone();
        }
        prepare(&output, args.profile.tree.as_deref()).await?;
        for (options, name) in [(&mut without, "without"), (&mut with, "with")] {
            options.tree = Some(output.join("input"));
            options.tree_out = Some(output.join(name).join("tree"));
            write_whole(&output.join(name).join("status.txt"), b"pending\n")?;
        }
        println!("comparison: {}", output.display());
        println!("without extra prompt: {}", output.join("without").display());
        println!("with extra prompt: {}", output.join("with").display());
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let runs = async {
            let without_dir = output.join("without");
            let with_dir = output.join("with");
            let first = run(cache, without, &args.prompt, &without_dir);
            let second = run(cache, with, &with_prompt, &with_dir);
            if args.sequential {
                (first.await, second.await)
            } else {
                tokio::join!(first, second)
            }
        };
        let results = tokio::select! {
            results = runs => Some(results),
            _ = tokio::signal::ctrl_c() => None,
            _ = terminate.recv() => None,
        };
        let Some((first, second)) = results else {
            for name in ["without", "with"] {
                let path = output.join(name).join("status.txt");
                if matches!(
                    std::fs::read_to_string(&path)?.trim(),
                    "pending" | "running"
                ) {
                    write_whole(&path, b"interrupted\n")?;
                }
            }
            anyhow::bail!(
                "interrupted; both VMs stopped; results kept in {}",
                output.display()
            );
        };
        let mut passed = true;
        for (name, result) in [("without", first), ("with", second)] {
            match result {
                Ok(true) => println!(
                    "{name}: completed; files in {}",
                    output.join(name).join("tree").display()
                ),
                Ok(false) => {
                    passed = false;
                    eprintln!("kai: {name}: incomplete; see status.txt and run/agent.json");
                }
                Err(error) => {
                    passed = false;
                    eprintln!("kai: {name}: {error:#}");
                }
            }
        }
        Ok::<_, anyhow::Error>(passed)
    })?;
    if passed {
        Ok(())
    } else {
        Err(Failure::Status(1))
    }
}

fn prompts(task: &str, extra: &str) -> anyhow::Result<String> {
    anyhow::ensure!(!task.trim().is_empty(), "the task must not be empty");
    anyhow::ensure!(
        !extra.trim().is_empty(),
        "the extra prompt must not be empty"
    );
    let with = format!("{extra}\n\n{task}");
    anyhow::ensure!(
        with.len() <= ::agent::session::MAX_PROMPT,
        "task and extra prompt together exceed 64 KiB"
    );
    Ok(with)
}

async fn prepare(output: &Path, tree: Option<&Path>) -> anyhow::Result<()> {
    let tree = tree
        .map(std::fs::canonicalize)
        .transpose()
        .context("cannot read input tree")?;
    if let Some(tree) = &tree {
        anyhow::ensure!(tree.is_dir(), "input tree must be a directory");
    }
    let parent = output.parent().context("output directory has no parent")?;
    std::fs::create_dir_all(parent)?;
    let parent = std::fs::canonicalize(parent)?;
    if let Some(tree) = &tree {
        anyhow::ensure!(
            !parent.starts_with(tree),
            "output must be outside the input tree"
        );
    }
    // Claim a new directory; never overwrite an earlier pair or its files.
    std::fs::create_dir(output).context("output must be a new directory")?;
    std::fs::create_dir(output.join("without"))?;
    std::fs::create_dir(output.join("with"))?;
    let input = output.join("input");
    if let Some(tree) = tree {
        // As for a VM's job drive: preserve links and modes, copy no target
        // of a link, and snapshot once before either session can start.
        helpers::process::run(
            tokio::process::Command::new("cp")
                .args(["-a", "--reflink=auto", "--"])
                .arg(tree.join("."))
                .arg(&input)
                .kill_on_drop(true),
        )
        .await?;
    } else {
        std::fs::create_dir(input)?;
    }
    Ok(())
}

async fn run(
    cache: &Path,
    options: ::agent::session::Options,
    prompt: &str,
    dir: &Path,
) -> anyhow::Result<bool> {
    write_whole(&dir.join("status.txt"), b"running\n")?;
    let result = execute(cache, options, Some(prompt), Some(&dir.join("answer.md"))).await;
    let status = match &result {
        Ok(true) => "completed\n".to_owned(),
        Ok(false) => "incomplete; see run/agent.json and run/result.json\n".to_owned(),
        Err(error) => format!("failed: {error:#}\n"),
    };
    write_whole(&dir.join("status.txt"), status.as_bytes())?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use helpers::testing::TestDir;

    #[test]
    fn the_task_is_unchanged_and_the_combined_prompt_is_bounded() {
        assert_eq!(
            prompts("build a calculator\n", "use one HTML file").unwrap(),
            "use one HTML file\n\nbuild a calculator\n"
        );
        assert!(prompts("", "extra").is_err());
        assert!(prompts("task", " \n").is_err());
        assert!(prompts(&"x".repeat(::agent::session::MAX_PROMPT), "extra").is_err());
    }

    #[tokio::test]
    async fn one_snapshot_keeps_the_source_and_existing_results_untouched() {
        let root = TestDir::new("compare-snapshot");
        let source = root.join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("index.html"), "initial").unwrap();
        let output = root.join("results");
        prepare(&output, Some(&source)).await.unwrap();
        std::fs::write(source.join("index.html"), "later").unwrap();
        assert_eq!(
            std::fs::read_to_string(output.join("input/index.html")).unwrap(),
            "initial"
        );
        assert!(prepare(&output, Some(&source)).await.is_err());
        assert!(
            prepare(&source.join("results"), Some(&source))
                .await
                .is_err()
        );
        assert!(!source.join("results").exists());
    }
}
