//! One-shot and live agent sessions use the same isolated SDK runtime.
use super::{Context, Failure};
use crate::settings;
use anyhow::Context as _;
use clap::{Args, Subcommand};
use helpers::lines::BoundedLines;
use std::path::{Path, PathBuf};

mod compare;

#[derive(Args)]
pub(crate) struct AgentArgs {
    /// VM cache containing the image and pinned agent runtime
    #[arg(long, global = true, env = "KAI_VM_CACHE", value_name = "DIR")]
    cache: Option<PathBuf>,
    #[command(subcommand)]
    command: AgentCommand,
}
#[derive(Subcommand)]
enum AgentCommand {
    /// Fetch and verify the pinned Claude, SDK and Node runtime layer, or with --addon an add-on's layer
    Fetch(FetchArgs),
    /// Run one prompt in a fresh microVM and print its answer
    Run(RunArgs),
    /// Run the same task twice, with and without an extra prompt, in parallel and with separate results
    Compare(compare::CompareArgs),
    /// Exchange JSON input commands and ordered output events with a fresh microVM; a refused command is a `refused` line and the session goes on; exits 0 when the session closed and its VM ended cleanly, whatever its turns' outcomes, which the result events and agent.json give
    Session(SessionArgs),
}
#[derive(Args)]
struct FetchArgs {
    /// Instead of the runtime: the add-on of this name, its pinned files packed as a read-only layer in a directory of its name under /opt/kai/addons/
    #[arg(long, value_name = "NAME", value_parser = clap::builder::PossibleValuesParser::new(::agent::addon::ADDONS.iter().map(|addon| addon.name)))]
    addon: Option<String>,
}
#[derive(Args)]
struct ProfileArgs {
    /// Explicit model identifier; no default model is selected
    #[arg(long)]
    model: String,
    /// Built image recipe
    #[arg(long, default_value = "rust")]
    image: String,
    /// Initial work tree, copied into the guest
    #[arg(long)]
    tree: Option<PathBuf>,
    /// Read-only layers below the runtime, bottom first
    #[arg(long = "layer")]
    layers: Vec<PathBuf>,
    /// Guest processors
    #[arg(long, default_value_t = 2)]
    cpus: u32,
    /// Guest memory in MiB
    #[arg(long, default_value_t = 4096)]
    memory: u32,
    /// Whole VM time limit, in seconds
    #[arg(long, default_value_t = 600)]
    timeout: u64,
    /// Comma-separated tool allowlist; an empty string disables tools
    #[arg(long, default_value = "Read,Edit,Write,Bash,Glob,Grep")]
    tools: String,
    /// The most model requests Claude makes for one turn, from 1; a turn that reaches it ends with the outcome `limit`. Without it, only the VM's time limit bounds a turn
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=i64::from(::agent::session::MAX_TURNS)))]
    max_turns: Option<u32>,
    /// Use a local fake model endpoint with a fixed dummy credential
    #[arg(long)]
    mock: bool,
}
#[derive(Args)]
struct SessionArgs {
    #[command(flatten)]
    profile: ProfileArgs,
    /// Export the final work tree to a new directory
    #[arg(long)]
    tree_out: Option<PathBuf>,
    /// Run directory, absent or empty
    #[arg(long)]
    run_dir: Option<PathBuf>,
    /// An add-on fetched before with `kai agent fetch --addon`: its layer is mounted and its directory given to Claude as a local plugin; may be given several times
    #[arg(long = "addon", value_name = "NAME", value_parser = clap::builder::PossibleValuesParser::new(::agent::addon::ADDONS.iter().map(|addon| addon.name)))]
    addons: Vec<String>,
    /// A slash command and its arguments, put on a line of its own before the first turn's text, such as "/ponytail full"; `agent run` fails when Claude does not take it
    #[arg(long, value_name = "TEXT")]
    activation: Option<String>,
}
#[derive(Args)]
struct RunArgs {
    #[command(flatten)]
    session: SessionArgs,
    /// Prompt, as one argument; never interpreted by a shell
    #[arg(last = true, required = true)]
    prompt: String,
}

pub(crate) fn dispatch(args: &AgentArgs, context: &Context<'_>) -> Result<(), Failure> {
    let cache =
        settings::vm_cache(args.cache.as_deref(), context.config).map_err(Failure::Usage)?;
    if let AgentCommand::Compare(args) = &args.command {
        return compare::dispatch(args, &cache, context);
    }
    if let AgentCommand::Fetch(FetchArgs { addon: Some(name) }) = &args.command {
        return fetch_addon(name, &cache, context);
    }
    if matches!(args.command, AgentCommand::Fetch(_)) {
        let fetched = context
            .runtime
            .block_on(::agent::runtime::fetch(&cache))
            .context("cannot fetch agent runtime")?;
        println!(
            "{}: {}",
            if fetched.reused {
                "verified runtime"
            } else {
                "prepared runtime"
            },
            fetched.file.display()
        );
        if let Some((path, reason)) = fetched.quarantined {
            eprintln!("kai: invalid runtime moved to {}: {reason}", path.display());
        }
        return Ok(());
    }
    let (session_args, prompt) = match &args.command {
        AgentCommand::Run(run) => (&run.session, Some(run.prompt.as_str())),
        AgentCommand::Session(session) => (session, None),
        AgentCommand::Fetch(_) | AgentCommand::Compare(_) => unreachable!(),
    };
    if prompt.is_some_and(|text| text.len() > ::agent::session::MAX_PROMPT) {
        return Err(Failure::Usage(anyhow::anyhow!("prompt exceeds 64 KiB")));
    }
    let run_dir = session_args
        .run_dir
        .clone()
        .unwrap_or_else(|| ::vm::new_run_dir(&cache, Some("agent")));
    let mut options = session_args.profile.options(run_dir)?;
    options.tree_out = session_args.tree_out.clone();
    options.activation = session_args.activation.clone();
    let passed = context.runtime.block_on(async {
        for name in &session_args.addons {
            let addon = ::agent::addon::named(name).expect("clap admits only known add-ons");
            options
                .addons
                .push(::agent::addon::cached(addon, &cache).await?);
        }
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = execute(&cache, options, prompt, None) => result,
            _ = tokio::signal::ctrl_c() => anyhow::bail!("interrupted; the VM was stopped"),
            _ = terminate.recv() => anyhow::bail!("terminated; the VM was stopped"),
        }
    })?;
    if passed {
        Ok(())
    } else {
        Err(Failure::Status(1))
    }
}

impl ProfileArgs {
    /// The same model, limits and input layers for every use of the profile.
    fn options(&self, run_dir: PathBuf) -> Result<::agent::session::Options, Failure> {
        // Never delegate env parsing to clap: its diagnostics can echo a secret.
        let endpoint = std::env::var("ANTHROPIC_BASE_URL").map_err(|_| {
            Failure::Usage(anyhow::anyhow!(
                "ANTHROPIC_BASE_URL must name a loopback model route"
            ))
        })?;
        let key = std::env::var("ANTHROPIC_API_KEY").ok();
        let route = ::agent::session::Route::new(&endpoint, key.as_deref(), self.mock)
            .map_err(|e| Failure::Usage(e.into()))?;
        let mut options = ::agent::session::Options::new(self.model.clone(), route, run_dir);
        options.image = self.image.clone();
        options.tree = self.tree.clone();
        options.layers = self.layers.clone();
        options.cpus = self.cpus;
        options.memory_mib = self.memory;
        options.timeout_s = self.timeout;
        options.tools = if self.tools.is_empty() {
            vec![]
        } else {
            self.tools.split(',').map(str::to_owned).collect()
        };
        options.max_turns = self.max_turns;
        Ok(options)
    }
}

/// `kai agent fetch --addon <name>`: the add-on's layer, made or checked.
fn fetch_addon(name: &str, cache: &Path, context: &Context<'_>) -> Result<(), Failure> {
    let addon = ::agent::addon::named(name).ok_or_else(|| {
        let known: Vec<&str> = ::agent::addon::ADDONS.iter().map(|a| a.name).collect();
        Failure::Usage(anyhow::anyhow!(
            "no add-on `{name}`; kai knows {}",
            known.join(", ")
        ))
    })?;
    let fetched = context
        .runtime
        .block_on(::agent::addon::fetch(addon, cache))
        .with_context(|| format!("cannot fetch the add-on {name}"))?;
    println!(
        "{} add-on {name}: {}",
        if fetched.reused {
            "verified"
        } else {
            "prepared"
        },
        fetched.file.display()
    );
    if let Some((path, reason)) = fetched.quarantined {
        eprintln!(
            "kai: invalid add-on layer moved to {}: {reason}",
            path.display()
        );
    }
    Ok(())
}

async fn execute(
    cache: &Path,
    options: ::agent::session::Options,
    prompt: Option<&str>,
    answer_file: Option<&Path>,
) -> anyhow::Result<bool> {
    let run_dir = options.run_dir.clone();
    let activation = options.activation.clone();
    let mut session = ::agent::session::Session::start(cache, options).await?;
    let passed;
    if let Some(prompt) = prompt {
        session.send(prompt).await?;
        loop {
            let event = session.next_event().await?;
            if event.kind == "result" {
                if let Some(text) = event.payload["result"].as_str() {
                    if let Some(path) = answer_file {
                        helpers::fs::write_whole(path, text.as_bytes())?;
                    } else {
                        println!("{text}");
                    }
                }
                break;
            }
        }
        let turn = session.turns().last();
        // An activation Claude did not take went to the model as text: the
        // turn may be answered, but it is not the turn that was asked for.
        let activated = match (&activation, turn) {
            (Some(activation), Some(turn)) => ::agent::session::invoked(turn, activation),
            _ => true,
        };
        if !activated {
            eprintln!(
                "kai: Claude did not take the activation's slash command; agent.json records the commands it took"
            );
        }
        if turn.is_some_and(|turn| turn.outcome == "limit") {
            eprintln!(
                "kai: the turn reached its limit of model requests; its last tool call may have changed the tree"
            );
        }
        passed = activated && turn.is_some_and(|turn| turn.outcome == "answered");
    } else {
        println!(
            "{}",
            serde_json::json!({"format": ::agent::runtime::FORMAT, "seq": 1, "kind":"ready", "payload":{"sdk": ::agent::runtime::SDK_VERSION,"node": ::agent::runtime::NODE_VERSION,"claude": ::agent::runtime::CLAUDE.version}})
        );
        // The protocol's frame limit, not the prompt's: a prompt of 64 KiB
        // can take several times that once escaped as JSON.
        let mut lines = BoundedLines::new(tokio::io::stdin(), ::agent::session::MAX_FRAME);
        let mut input_failed = false;
        loop {
            tokio::select! {
                line = lines.next() => {
                    let line = match line {
                        Ok(Some(line)) => line,
                        Ok(None) => break,
                        // Past the frame limit, or cut off by the end of the
                        // input: the stream cannot be read on, so the session
                        // is closed as a close command would.
                        Err(err) => {
                            eprintln!("kai: {err}; closing the session");
                            input_failed = true;
                            break;
                        }
                    };
                    match input(&mut session, &line).await? {
                        Ok(Next::Continue) => {}
                        Ok(Next::Close) => break,
                        Err(reason) => println!(
                            "{}",
                            serde_json::json!({"format": ::agent::runtime::FORMAT, "kind": "refused", "payload": {"reason": reason}})
                        ),
                    }
                },
                event = session.next_event() => { println!("{}", serde_json::to_string(&event?)?); },
            }
        }
        // The session's own outcome: whether it closed and its VM ended
        // cleanly. Each turn's outcome is in its result event and agent.json.
        passed = !input_failed;
    }
    let result = session
        .close_with(|event| {
            if prompt.is_none() {
                println!(
                    "{}",
                    serde_json::to_string(event).expect("event fields serialize")
                );
            }
        })
        .await?;
    eprintln!("kai: agent run saved in {}", run_dir.display());
    Ok(passed
        && result.outcome == ::vm::Outcome::Exited
        && result.exit_code == Some(0)
        && result.tree_out != Some(false))
}

#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
enum Input {
    Send { text: String },
    Interrupt,
    Close,
}

/// What the interactive loop does after an input command.
enum Next {
    Continue,
    Close,
}

/// One input command, carried out. A command the session refuses without
/// changing anything (not JSON or not a command, a send while a turn is
/// active, an interrupt without one or twice) is `Err` with a reason that
/// echoes nothing of the input, and the session goes on; any other error
/// has ended the session.
async fn input(
    session: &mut ::agent::session::Session,
    line: &[u8],
) -> anyhow::Result<Result<Next, &'static str>> {
    let Ok(command) = serde_json::from_slice::<Input>(line) else {
        return Ok(Err("invalid session input command"));
    };
    let done = match command {
        Input::Send { text } => session.send(&text).await.map(drop),
        Input::Interrupt => session.interrupt().await,
        Input::Close => return Ok(Ok(Next::Close)),
    };
    match done {
        Ok(()) => Ok(Ok(Next::Continue)),
        Err(::agent::session::Error::Invalid(reason)) => Ok(Err(reason)),
        Err(error) => Err(error.into()),
    }
}
