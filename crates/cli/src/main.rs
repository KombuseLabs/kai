//! kai's command line: reads arguments, calls the library crates, prints.
//!
//! - `kai` greets.
//! - `kai github sync` fetches a board's issues into the store, a SQLite
//!   cache of GitHub. It never writes to GitHub.
//! - `kai ticket list` lists the tickets in the store.
//! - `kai ticket qualify` says for each whether kai may use it, judged from
//!   what GitHub said: qualified, refused, or undecided while a rule is not
//!   checked.
//! - `kai store reset` empties the store; the next sync fetches everything.
//! - `kai study estimate` says from a study file's size what the study can
//!   see and its worst-case token use; it spends nothing.
//! - `kai vm doctor` says what running a microVM needs on this machine and
//!   what fixes each gap; `kai vm fetch` downloads the pinned Firecracker
//!   and guest kernel into kai's cache and checks them; `kai vm run` runs
//!   one command in a fresh microVM on an image file.
//! - `kai agent fetch` downloads the pinned Claude Code, checks it and
//!   packs it as a read-only layer for a microVM, in the same cache.
//! - `kai agent compare` runs one task with and without extra instructions,
//!   in parallel by default, and keeps both result trees and answers.
//!
//! Results go to standard output, log lines never do: they go to standard
//! error, at the level `--log-level` names, and with `--log-file` also to a
//! file, as one JSON object per line. `kai --help` lists the options.
//!
//! Settings: the first `kai.toml` from the current directory upward, or the
//! file `--config` names. An option, or the environment variable beside it,
//! wins over the file, and the file over kai's default. A path in the file
//! is taken from the file's directory.
//!
//! A `.env` file, the first from the current directory upward, is read into
//! the environment before anything else, so that the `KAI_*` variables can
//! stand in it. A variable that is already set wins over the file. The file
//! is for secrets and stays out of version control.
//!
//! The GitHub token: `KAI_GITHUB_TOKEN`, else what `gh auth token` prints.
//! It is never an option, so that it stays out of the shell's history and
//! the process list.
//!
//! Exit status: 0; 1 when a command fails (GitHub, the store, a download,
//! a need `kai vm doctor` finds unmet); 2 for a command line, a `.env` or a
//! settings file kai cannot read or that lacks what the command needs, or
//! when logging cannot be set up. `kai vm run` exits with its command's
//! status; with 125 when the command has none (a time limit, a signal, a
//! VM that failed), and with 130 when it was interrupted.
//!
//! Here: starting up, the options every command takes, the exit status.
//! [`settings`]: kai.toml and which setting wins. [`cmd`]: the commands, a
//! module per group.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "printing is what the command line is for"
)]

mod cmd;
mod settings;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context as _;
use clap::Parser;
use helpers::logging;

use cmd::{Command, Context, Failure};
use settings::{CONFIG_FILE, LogLevel};

/// Compares variants of a coding agent on your own tickets.
#[derive(Parser)]
#[command(name = "kai", version)]
struct Cli {
    /// Read the settings from this file instead of the first kai.toml from the current directory upward
    #[arg(long, global = true, env = "KAI_CONFIG", value_name = "FILE")]
    config: Option<PathBuf>,

    /// Lowest level of the log lines shown on standard error; without it, log.level in kai.toml, else warn
    #[arg(long, global = true, env = "KAI_LOG_LEVEL", value_name = "LEVEL")]
    log_level: Option<LogLevel>,

    /// Also write every log line from debug up to this file, one JSON object per line; without it, log.file in kai.toml
    #[arg(long, global = true, env = "KAI_LOG_FILE", value_name = "FILE")]
    log_file: Option<PathBuf>,

    /// The store, a SQLite cache of the tickets; without it, store.path in kai.toml, else .kai/store.sqlite beside kai.toml or here
    #[arg(long, global = true, env = "KAI_STORE", value_name = "FILE")]
    store: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

/// Reads the first `.env` from the current directory upward into the
/// environment. No such file is fine; one that cannot be read or parsed is
/// not, because a secret that silently failed to load is hard to find.
fn load_dotenv() -> anyhow::Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(err) if err.not_found() => Ok(()),
        // dotenvy's message quotes the line, and the line may hold a secret.
        Err(dotenvy::Error::LineParse(..)) => {
            anyhow::bail!(
                "cannot read .env: a line is malformed (not shown, as it may hold a secret)"
            )
        }
        Err(err) => Err(err).context("cannot read .env"),
    }
}

fn main() -> ExitCode {
    // Before `Cli::parse`: clap reads the KAI_* variables from the
    // environment while it parses.
    if let Err(err) = load_dotenv() {
        eprintln!("kai: {err:#}");
        return ExitCode::from(2);
    }
    let cli = Cli::parse();
    let config = match settings::load(cli.config.as_deref()) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("kai: {err:#}");
            return ExitCode::from(2);
        }
    };
    let sinks = settings::sinks(cli.log_level, cli.log_file.as_deref(), config.as_ref());
    // Kept until main returns: dropping it writes what is still queued.
    let _logging = match logging::init(sinks) {
        Ok(logging) => logging,
        Err(err) => {
            eprintln!("kai: cannot set up logging: {err}");
            return ExitCode::from(2);
        }
    };
    tracing::debug!(version = env!("CARGO_PKG_VERSION"), "kai started");
    match &config {
        Some(config) => tracing::debug!(file = %config.path.display(), "settings read"),
        None => tracing::debug!("no {CONFIG_FILE} found; kai's defaults apply"),
    }

    // The one async runtime, on a single thread: a command waits on the
    // network or on a VM, and on little else at the same time.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("kai: cannot start the async runtime: {err}");
            return ExitCode::from(1);
        }
    };
    let context = Context {
        store: settings::store_path(cli.store.as_deref(), config.as_ref()),
        config: config.as_ref(),
        runtime: &runtime,
    };
    match cmd::run(cli.command.as_ref(), &context) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(err)) => {
            eprintln!("kai: {err:#}");
            ExitCode::from(2)
        }
        Err(Failure::Run(err)) => {
            eprintln!("kai: {err:#}");
            ExitCode::from(1)
        }
        Err(Failure::Status(status)) => ExitCode::from(status),
    }
}
