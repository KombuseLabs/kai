//! kai's command line: reads arguments, calls `engine`, prints. For now it
//! only greets.
//!
//! Results go to standard output, log lines never do: they go to standard
//! error, at the level `--log-level` names, and with `--log-file` also to a
//! file, as one JSON object per line. `kai --help` lists the options.
//!
//! Exit status: 0; 2 for a command line kai cannot read, or when logging
//! cannot be set up.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "printing is what the command line is for"
)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use helpers::logging::{self, Format, Sink, Target};
use tracing::Level;

/// Compares variants of a coding agent on your own tickets.
#[derive(Parser)]
#[command(name = "kai", version)]
struct Cli {
    /// Lowest level of the log lines shown on standard error
    #[arg(
        long,
        env = "KAI_LOG_LEVEL",
        value_name = "LEVEL",
        default_value = "warn"
    )]
    log_level: LogLevel,

    /// Also write every log line from debug up to this file, one JSON object per line
    #[arg(long, env = "KAI_LOG_FILE", value_name = "FILE")]
    log_file: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
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

impl Cli {
    /// The sinks the options ask for.
    fn sinks(&self) -> Vec<Sink> {
        let mut sinks = Vec::new();
        if let Some(level) = self.log_level.level() {
            sinks.push(Sink::new(Target::Stderr, Format::Pretty, level));
        }
        if let Some(path) = &self.log_file {
            let file = Target::File(path.clone());
            sinks.push(Sink::new(file, Format::Json, Level::DEBUG));
        }
        sinks
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // Kept until main returns: dropping it writes what is still queued.
    let _logging = match logging::init(cli.sinks()) {
        Ok(logging) => logging,
        Err(err) => {
            eprintln!("kai: cannot set up logging: {err}");
            return ExitCode::from(2);
        }
    };
    tracing::debug!(version = env!("CARGO_PKG_VERSION"), "kai started");

    println!("{}", engine::greeting());
    ExitCode::SUCCESS
}
