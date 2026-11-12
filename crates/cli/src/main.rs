//! kai's command line: reads arguments, calls `engine`, prints. For now it
//! only greets.
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
//! Exit status: 0; 2 for a command line or a settings file kai cannot read,
//! or when logging cannot be set up.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "printing is what the command line is for"
)]

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context as _;
use clap::{Parser, ValueEnum};
use helpers::config::{self, Loaded};
use helpers::logging::{self, Format, Sink, Target};
use serde::Deserialize;
use tracing::Level;

/// The settings file kai looks for, from the current directory upward.
const CONFIG_FILE: &str = "kai.toml";

/// Compares variants of a coding agent on your own tickets.
#[derive(Parser)]
#[command(name = "kai", version)]
struct Cli {
    /// Read the settings from this file instead of the first kai.toml from the current directory upward
    #[arg(long, env = "KAI_CONFIG", value_name = "FILE")]
    config: Option<PathBuf>,

    /// Lowest level of the log lines shown on standard error; without it, log.level in kai.toml, else warn
    #[arg(long, env = "KAI_LOG_LEVEL", value_name = "LEVEL")]
    log_level: Option<LogLevel>,

    /// Also write every log line from debug up to this file, one JSON object per line; without it, log.file in kai.toml
    #[arg(long, env = "KAI_LOG_FILE", value_name = "FILE")]
    log_file: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// `None` for `off`.
    fn level(self) -> Option<Level> {
        match self {
            LogLevel::Off => None,
            LogLevel::Error => Some(Level::ERROR),
            LogLevel::Warn => Some(Level::WARN),
            LogLevel::Info => Some(Level::INFO),
            LogLevel::Debug => Some(Level::DEBUG),
            LogLevel::Trace => Some(Level::TRACE),
        }
    }
}

/// What a `kai.toml` may hold. Every key may be left out; a key kai does
/// not know is refused, so that a misspelt one is never silently ignored.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    log: LogConfig,
}

/// The `[log]` table, as `--log-level` and `--log-file`.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogConfig {
    level: Option<LogLevel>,
    file: Option<PathBuf>,
}

impl Cli {
    /// The settings file: the one `--config` names, else the first
    /// `kai.toml` from the current directory upward, else none.
    fn config(&self) -> anyhow::Result<Option<Loaded<Config>>> {
        let path = match &self.config {
            Some(path) => path.clone(),
            None => {
                let here = std::env::current_dir().context("cannot tell the current directory")?;
                match config::find(&here, CONFIG_FILE) {
                    Some(path) => path,
                    None => return Ok(None),
                }
            }
        };
        Ok(Some(config::load(&path)?))
    }

    /// The sinks the options and the settings file ask for.
    fn sinks(&self, config: Option<&Loaded<Config>>) -> Vec<Sink> {
        let log = config.map(|config| &config.value.log);
        let level = self
            .log_level
            .or(log.and_then(|log| log.level))
            .unwrap_or(LogLevel::Warn);
        let file = self.log_file.clone().or_else(|| {
            let config = config?;
            let file = config.value.log.file.as_deref()?;
            Some(config.resolve(file))
        });

        let mut sinks = Vec::new();
        if let Some(level) = level.level() {
            sinks.push(Sink::new(Target::Stderr, Format::Pretty, level));
        }
        if let Some(path) = file {
            sinks.push(Sink::new(Target::File(path), Format::Json, Level::DEBUG));
        }
        sinks
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let config = match cli.config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("kai: {err:#}");
            return ExitCode::from(2);
        }
    };
    // Kept until main returns: dropping it writes what is still queued.
    let _logging = match logging::init(cli.sinks(config.as_ref())) {
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

    println!("{}", engine::greeting());
    ExitCode::SUCCESS
}
