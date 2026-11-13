//! kai.toml, and which wins over which: an option, or the environment
//! variable beside it, over the file, and the file over kai's default.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::ValueEnum;
use helpers::config::{self, Loaded};
use helpers::logging::{Format, Sink, Target};
use serde::Deserialize;
use tracing::Level;
use types::RepoName;

/// The settings file kai looks for, from the current directory upward.
pub(crate) const CONFIG_FILE: &str = "kai.toml";

/// Where the store is when nothing says otherwise: taken from the
/// directory of the settings file, else from the current one.
const DEFAULT_STORE: &str = ".kai/store.sqlite";

/// What a `kai.toml` may hold. Every key may be left out; a key kai does
/// not know is refused, so that a misspelt one is never silently ignored.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    #[serde(default)]
    log: LogConfig,
    #[serde(default)]
    github: GithubConfig,
    #[serde(default)]
    store: StoreConfig,
}

/// The `[log]` table, as `--log-level` and `--log-file`.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogConfig {
    level: Option<LogLevel>,
    file: Option<PathBuf>,
}

/// The `[github]` table.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct GithubConfig {
    /// As `--board`.
    board: Option<String>,
    /// The API's URL; github.com's when unset.
    api: Option<String>,
}

/// The `[store]` table, as `--store`.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreConfig {
    path: Option<PathBuf>,
}

/// The lowest level of the log lines on standard error.
#[derive(Clone, Copy, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LogLevel {
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

/// The settings file: the one `path` names (`--config`), else the first
/// `kai.toml` from the current directory upward, else none.
pub(crate) fn load(path: Option<&Path>) -> anyhow::Result<Option<Loaded<Config>>> {
    let path = match path {
        Some(path) => path.to_path_buf(),
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

/// The sinks `--log-level`, `--log-file` and the settings file ask for.
pub(crate) fn sinks(
    level: Option<LogLevel>,
    file: Option<&Path>,
    config: Option<&Loaded<Config>>,
) -> Vec<Sink> {
    let log = config.map(|config| &config.value.log);
    let level = level
        .or(log.and_then(|log| log.level))
        .unwrap_or(LogLevel::Warn);
    let file = file.map(Path::to_path_buf).or_else(|| {
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

/// The store's file: `store` (`--store`), else `store.path` in the
/// settings file, else the default beside the settings file, else the
/// default here.
pub(crate) fn store_path(store: Option<&Path>, config: Option<&Loaded<Config>>) -> PathBuf {
    if let Some(path) = store {
        return path.to_path_buf();
    }
    match config {
        Some(config) => config.resolve(
            config
                .value
                .store
                .path
                .as_deref()
                .unwrap_or(Path::new(DEFAULT_STORE)),
        ),
        None => PathBuf::from(DEFAULT_STORE),
    }
}

/// The board: `board` (`--board` or `KAI_GITHUB_BOARD`), else
/// `github.board` in the settings file.
pub(crate) fn board(
    board: Option<&RepoName>,
    config: Option<&Loaded<Config>>,
) -> anyhow::Result<RepoName> {
    if let Some(board) = board {
        return Ok(board.clone());
    }
    let Some((config, board)) =
        config.and_then(|config| Some((config, config.value.github.board.as_deref()?)))
    else {
        anyhow::bail!(
            "no board to sync: give --board, set KAI_GITHUB_BOARD, or set github.board in {CONFIG_FILE}"
        );
    };
    board
        .parse()
        .with_context(|| format!("github.board in {}", config.path.display()))
}

/// The GitHub API: `github.api` in the settings file, else github.com's.
pub(crate) fn api(config: Option<&Loaded<Config>>) -> &str {
    config
        .and_then(|config| config.value.github.api.as_deref())
        .unwrap_or(github::API)
}
