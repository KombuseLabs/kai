//! `kai vm`: microVMs. What running one needs on this machine, the pinned
//! Firecracker and guest kernel, and one command in a fresh VM.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::{Args, Subcommand};

use tokio::runtime::Runtime;

use super::{Context, Failure};
use crate::settings;

#[derive(Args)]
pub(crate) struct VmArgs {
    /// Where Firecracker and the guest kernel are kept; without it, vm.cache in kai.toml, else ~/.cache/kai/vm
    #[arg(long, global = true, env = "KAI_VM_CACHE", value_name = "DIR")]
    cache: Option<PathBuf>,

    #[command(subcommand)]
    command: VmCommand,
}

#[derive(Subcommand)]
enum VmCommand {
    /// Says what running a microVM needs on this machine, whether each need is met, and what fixes it; exits with 1 while one is not
    Doctor,
    /// Downloads the pinned Firecracker and guest kernel into the cache and checks each against its pinned SHA-256; a file that does not match is not kept
    Fetch,
    /// Images from the recipes kai ships
    #[command(subcommand)]
    Image(ImageCommand),
    /// Runs one command in a fresh microVM without network, then prints what it wrote to each stream and exits with its status (125 when it has none)
    Run(Box<RunArgs>),
}

#[derive(Subcommand)]
enum ImageCommand {
    /// Builds the image of a recipe with Docker, packs it, checks it boots, and prints its id; needs `kai vm fetch` first
    Build {
        /// The recipe's name; `kai vm image list` lists them
        recipe: String,
    },
    /// Lists the recipes, their image ids, and which are built
    List,
}

#[derive(Args)]
struct RunArgs {
    /// The image: a recipe's name (its image must be built), an image id (sha256:…), or a squashfs or ext4 file
    #[arg(long, value_name = "IMAGE")]
    image: String,

    /// Adds NAME=VALUE to the command's environment, which otherwise is the image's (for an image file: only PATH); may be given several times
    // Taken as text and checked by `environment`: clap's own error would
    // show the entry, which may hold a secret.
    #[arg(long = "env", value_name = "NAME=VALUE")]
    env: Vec<String>,

    /// The command's directory in the guest
    #[arg(long, value_name = "PATH", default_value = ::vm::DEFAULT_CWD)]
    cwd: String,

    /// Processors of the VM, 1 to 32 (Firecracker's range)
    #[arg(long, value_name = "N", default_value_t = ::vm::DEFAULT_CPUS,
          value_parser = clap::value_parser!(u32).range(1..=32))]
    cpus: u32,

    /// Memory of the VM in MiB, at least 128 (a kernel boots in no less)
    #[arg(long, value_name = "MIB", default_value_t = ::vm::DEFAULT_MEMORY_MIB,
          value_parser = clap::value_parser!(u32).range(128..))]
    memory: u32,

    /// Seconds the command may run before the guest ends it
    #[arg(long, value_name = "SECONDS", default_value_t = ::vm::DEFAULT_TIMEOUT_S,
          value_parser = clap::value_parser!(u64).range(1..))]
    timeout: u64,

    /// The run directory, which must not exist or be empty; without it, a new one under runs/ in the cache
    #[arg(long, value_name = "DIR")]
    run_dir: Option<PathBuf>,

    /// A directory copied in as the work tree, /job/tree; it is never changed here
    #[arg(long, value_name = "DIR")]
    tree: Option<PathBuf>,

    /// Where the tree is written after the run, as the command left it; must not exist
    #[arg(long, value_name = "DIR")]
    tree_out: Option<PathBuf>,

    /// Copies a file or directory of this machine onto the job drive, to a path under /job; may be given several times
    #[arg(long = "file", value_name = "HOST_PATH=JOB_PATH", value_parser = parse_file)]
    files: Vec<(PathBuf, String)>,

    /// A read-only layer over the image: a file an earlier --upper-out kept; may be given several times, the last on top
    #[arg(long = "layer", value_name = "FILE")]
    layers: Vec<PathBuf>,

    /// Keeps what the command wrote outside its tree as a layer in this file, when it exits with 0; must not exist
    #[arg(long, value_name = "FILE")]
    upper_out: Option<PathBuf>,

    /// Size of the job drive in MiB; without it, twice the tree and files, plus 2048
    #[arg(long, value_name = "MIB")]
    disk: Option<u64>,

    /// The user the command runs as in the guest, as UID:GID, who then owns the tree and a home, /job/home; without it, root
    #[arg(long, value_name = "UID:GID", value_parser = parse_user)]
    user: Option<(u32, u32)>,

    /// A way out: the guest's 127.0.0.1:N reaches a Unix socket (N=unix:PATH) or a TCP address (N=tcp:HOST:PORT) of this machine; may be given several times
    #[arg(long = "port", value_name = "N=unix:PATH|N=tcp:HOST:PORT", value_parser = parse_port)]
    ports: Vec<::vm::Port>,

    /// A way out through `fence proxy` on the guest's 127.0.0.1:3128, with HTTPS_PROXY and the rest set, to HOST:PORT (fence's net.allow syntax); may be given several times. Needs fence 0.3.0 or newer on PATH
    #[arg(long = "proxy", value_name = "HOST:PORT")]
    proxy: Vec<String>,

    /// With --proxy: fence's learn mode; every host is let through and those --proxy does not name are recorded; refused together with --upper-out
    #[arg(long, requires = "proxy", conflicts_with = "upper_out")]
    proxy_learn: bool,

    /// The program and its arguments, after `--`
    #[arg(last = true, required = true, value_name = "COMMAND")]
    command: Vec<String>,
}

/// `HOST_PATH=JOB_PATH`, split at the last `=`.
fn parse_file(text: &str) -> Result<(PathBuf, String), String> {
    let (from, to) = text
        .rsplit_once('=')
        .ok_or_else(|| format!("`{text}` is not HOST_PATH=JOB_PATH"))?;
    if from.is_empty() || to.is_empty() {
        return Err(format!("`{text}` is not HOST_PATH=JOB_PATH"));
    }
    Ok((PathBuf::from(from), to.to_owned()))
}

/// `N=unix:PATH` or `N=tcp:HOST:PORT`.
fn parse_port(text: &str) -> Result<::vm::Port, String> {
    let bad = || format!("`{text}` is not N=unix:PATH or N=tcp:HOST:PORT");
    let (guest, to) = text.split_once('=').ok_or_else(bad)?;
    let guest: u16 = guest.parse().map_err(|_| bad())?;
    let to = if let Some(path) = to.strip_prefix("unix:") {
        ::vm::PortTarget::Unix(PathBuf::from(path))
    } else if let Some(address) = to.strip_prefix("tcp:") {
        ::vm::PortTarget::Tcp(address.to_owned())
    } else {
        return Err(bad());
    };
    if guest == 0 {
        return Err(bad());
    }
    Ok(::vm::Port { guest, to })
}

/// `UID:GID`, two numbers.
fn parse_user(text: &str) -> Result<(u32, u32), String> {
    let (uid, gid) = text
        .split_once(':')
        .ok_or_else(|| format!("`{text}` is not UID:GID"))?;
    let number = |part: &str| {
        part.parse::<u32>()
            .map_err(|_| format!("`{text}` is not UID:GID with two numbers"))
    };
    Ok((number(uid)?, number(gid)?))
}

/// The `--env` entries as names and values: each `NAME=VALUE`, with a name
/// a shell would take. An entry may hold a secret, so a refusal names the
/// entry by its place and shows nothing of it.
fn environment(entries: &[String]) -> anyhow::Result<Vec<(String, String)>> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let place = index + 1;
            let Some((name, value)) = entry.split_once('=') else {
                anyhow::bail!(
                    "--env #{place} has no `=`; an entry is NAME=VALUE (not shown, as it may hold a secret)"
                );
            };
            let valid = !name.is_empty()
                && !name.starts_with(|c: char| c.is_ascii_digit())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            anyhow::ensure!(
                valid,
                "--env #{place}: the name before `=` is not a variable name: letters, digits and _, not a digit first (not shown, as it may hold a secret)"
            );
            Ok((name.to_owned(), value.to_owned()))
        })
        .collect()
}

pub(crate) fn dispatch(args: &VmArgs, context: &Context<'_>) -> Result<(), Failure> {
    let cache =
        settings::vm_cache(args.cache.as_deref(), context.config).map_err(Failure::Usage)?;
    let runtime = context.runtime;
    match &args.command {
        VmCommand::Doctor => doctor(&cache),
        VmCommand::Fetch => fetch(&cache, runtime),
        VmCommand::Image(ImageCommand::Build { recipe }) => image_build(recipe, &cache, runtime),
        VmCommand::Image(ImageCommand::List) => image_list(&cache),
        VmCommand::Run(run_args) => run(run_args, &cache, runtime),
    }
}

/// Builds the image of `name` and prints its id and file.
fn image_build(name: &str, cache: &Path, runtime: &Runtime) -> Result<(), Failure> {
    let Some(recipe) = ::vm::image::recipe(name) else {
        return Err(Failure::Usage(anyhow::anyhow!(
            "kai has no recipe `{name}`; `kai vm image list` lists them"
        )));
    };
    let info = runtime.block_on(async {
        let launch = ::vm::Launch::from_cache(cache).await?;
        ::vm::image::build(recipe, cache, &launch).await
    });
    let (file, info, built) =
        info.with_context(|| format!("cannot build the image of `{name}`"))?;
    let what = match built {
        ::vm::image::Built::AlreadyThere => "already built",
        ::vm::image::Built::Now => "built and checked",
    };
    println!("{}: {what}, {} {}", info.recipe, info.id, file.display());
    Ok(())
}

/// One line per recipe: its name, its image id, and whether it is built.
fn image_list(cache: &Path) -> Result<(), Failure> {
    for recipe in ::vm::image::RECIPES {
        let id = ::vm::image::image_id(recipe);
        match ::vm::image::built(recipe, cache).context("cannot read the cache")? {
            Some((file, _)) => {
                // A size that cannot be read is unknown, never 0.
                let size = std::fs::metadata(&file).map_or_else(
                    |_| "size unknown".to_owned(),
                    |meta| format!("{} MB", meta.len() / 1_000_000),
                );
                println!("{}  {id}  built, {size}, {}", recipe.name, file.display());
            }
            None => println!(
                "{}  {id}  not built; `kai vm image build {}` builds it",
                recipe.name, recipe.name
            ),
        }
    }
    Ok(())
}

/// Prints one line per need: `ok` or `missing`, the need, and what was
/// found or what fixes it.
fn doctor(cache: &Path) -> Result<(), Failure> {
    let checks = ::vm::doctor::check(cache);
    for check in &checks {
        let state = match (check.met, check.optional) {
            (true, _) => "ok",
            (false, true) => "optional",
            (false, false) => "missing",
        };
        println!("{state:<8} {}: {}", check.need, check.detail);
    }
    let unmet = checks
        .iter()
        .filter(|check| !check.met && !check.optional)
        .count();
    if unmet > 0 {
        return Err(Failure::Run(anyhow::anyhow!(
            "{unmet} of {} needs are not met",
            checks.len()
        )));
    }
    Ok(())
}

/// Fetches every pinned artefact into `cache`, one after the other, and
/// says for each what happened. The first failure stops it.
fn fetch(cache: &Path, runtime: &Runtime) -> Result<(), Failure> {
    for artefact in &::vm::ARTEFACTS {
        let fetched = runtime
            .block_on(::vm::fetch(artefact, cache))
            .with_context(|| format!("cannot fetch {}", artefact.name))?;
        let what = match fetched {
            ::vm::Fetched::AlreadyThere => "already there, checked",
            ::vm::Fetched::Downloaded => "downloaded, checked",
        };
        println!(
            "{}: {what}, {}",
            artefact.name,
            cache.join(artefact.file).display()
        );
    }
    Ok(())
}

/// Runs the command, passes its output on, and exits as it did. Ctrl-C or
/// SIGTERM stops the VM before kai ends.
fn run(args: &RunArgs, cache: &Path, runtime: &Runtime) -> Result<(), Failure> {
    let entries = environment(&args.env).map_err(Failure::Usage)?;
    let (image, info) = match ::vm::image::resolve(&args.image, cache) {
        Ok(found) => found,
        Err(err) => {
            eprintln!("kai: no run: {err}");
            return Err(Failure::Status(125));
        }
    };
    // The image's environment, which its export dropped, under the
    // caller's.
    let mut env = match &info {
        Some(info) => info.environment(),
        None => BTreeMap::from([("PATH".to_owned(), ::vm::DEFAULT_PATH.to_owned())]),
    };
    env.extend(entries);
    let run_dir = match &args.run_dir {
        Some(dir) => dir.clone(),
        None => ::vm::new_run_dir(cache, None),
    };
    let mut spec = ::vm::RunSpec::new(image, args.command.clone(), run_dir.clone());
    spec.origin = info.as_ref().map(::vm::image::ImageInfo::origin);
    spec.env = env;
    spec.cwd = args.cwd.clone();
    spec.cpus = args.cpus;
    spec.memory_mib = args.memory;
    spec.timeout_s = args.timeout;
    spec.disk_mib = args.disk;
    spec.tree = args.tree.clone();
    spec.tree_out = args.tree_out.clone();
    spec.files = args
        .files
        .iter()
        .map(|(from, to)| ::vm::JobFile {
            from: from.clone(),
            to: to.clone(),
        })
        .collect();
    spec.layers = args.layers.clone();
    spec.upper_out = args.upper_out.clone();
    if let Some((uid, gid)) = args.user {
        spec.uid = uid;
        spec.gid = gid;
    }
    spec.ports = args.ports.clone();
    if !args.proxy.is_empty() {
        let mut policy = ::proxy::Policy::new(args.proxy.iter().cloned());
        policy.learn = args.proxy_learn;
        spec.proxy = Some(policy);
    }

    let ran = runtime.block_on(async {
        let launch = match ::vm::Launch::from_cache(cache).await {
            Ok(launch) => launch,
            Err(err) => return Some(Err(err)),
        };
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(terminate) => terminate,
                Err(err) => {
                    return Some(Err(::vm::Error::Io {
                        path: run_dir.clone(),
                        source: err,
                    }));
                }
            };
        tokio::select! {
            result = ::vm::run(&launch, &spec) => Some(result),
            _ = tokio::signal::ctrl_c() => None,
            _ = terminate.recv() => None,
        }
    });
    let result = match ran {
        None => {
            eprintln!(
                "kai: interrupted; the VM was stopped. {}",
                run_dir.display()
            );
            return Err(Failure::Status(130));
        }
        Some(Err(::vm::Error::Refused(why))) => {
            return Err(Failure::Usage(anyhow::anyhow!("no run: {why}")));
        }
        Some(Err(err)) => {
            eprintln!("kai: no run: {err}");
            return Err(Failure::Status(125));
        }
        Some(Ok(result)) => result,
    };

    pass_on(&run_dir.join("out/stdout"), &mut std::io::stdout());
    pass_on(&run_dir.join("out/stderr"), &mut std::io::stderr());
    tracing::info!(run = %run_dir.display(), outcome = ?result.outcome, "the run ended");
    if let (Some(target), Some(false)) = (&args.upper_out, result.upper_out) {
        eprintln!(
            "kai: {} was not kept: only a command that exits with 0 leaves a layer",
            target.display()
        );
    }
    if let Some(proxy) = &result.proxy {
        let refused: Vec<String> = proxy
            .refused_targets
            .iter()
            .map(|(target, count)| format!("{target} ({count})"))
            .collect();
        let learned: Vec<String> = proxy
            .would_deny_targets
            .iter()
            .map(|(target, count)| format!("{target} ({count})"))
            .collect();
        eprintln!(
            "kai: the egress let {} connections through and refused {}{}{}; {}",
            proxy.allowed,
            proxy.refused,
            if refused.is_empty() {
                String::new()
            } else {
                format!(": {}", refused.join(", "))
            },
            if learned.is_empty() {
                String::new()
            } else {
                format!(
                    "; learn mode let through what no rule names: {}",
                    learned.join(", ")
                )
            },
            run_dir.join("proxy.jsonl").display()
        );
    }
    if let (Some(target), Some(false)) = (&args.tree_out, result.tree_out) {
        eprintln!(
            "kai: the tree could not be written to {}; the job drive in {} holds it",
            target.display(),
            run_dir.display()
        );
    }
    match (result.outcome, result.exit_code) {
        (::vm::Outcome::Exited, Some(0)) => Ok(()),
        (::vm::Outcome::Exited, Some(code)) => {
            Err(Failure::Status(u8::try_from(code).unwrap_or(125)))
        }
        (outcome, _) => {
            let what = match outcome {
                ::vm::Outcome::Signalled => format!(
                    "the command was ended by signal {}",
                    result
                        .signal
                        .map_or("?".to_owned(), |signal| signal.to_string())
                ),
                ::vm::Outcome::Timeout => {
                    format!(
                        "the command ran past its time limit of {} s",
                        result.timeout_s
                    )
                }
                _ => format!(
                    "{}: {}",
                    outcome.name(),
                    result.reason.as_deref().unwrap_or("no reason recorded")
                ),
            };
            eprintln!("kai: {what}. {}", run_dir.display());
            Err(Failure::Status(125))
        }
    }
}

/// Copies the file at `path` to `to`, when it is a plain file: what came
/// back from the guest may be anything, a link to a file of this machine
/// included.
fn pass_on(path: &Path, to: &mut dyn std::io::Write) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if !meta.file_type().is_file() {
        return;
    }
    if let Ok(mut file) = std::fs::File::open(path) {
        let _ = std::io::copy(&mut file, to);
        let _ = to.flush();
    }
}
