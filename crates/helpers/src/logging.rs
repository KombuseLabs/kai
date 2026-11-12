//! Logging for kai's programs: code logs through `tracing`'s macros, and the
//! program decides at its start where the lines go.
//!
//! - **Sinks.** A [`Sink`] is a target (standard error, a file, or any
//!   writer, for example one that ships lines to a collector on another
//!   machine), a format (for people, or one JSON object per line) and the
//!   lowest level it takes. A program installs as many as it wants.
//! - **Never in the way.** The thread that logs formats its line and puts
//!   it into the sink's queue; one thread per sink writes. When a queue is
//!   full the line is dropped and counted ([`Logging::dropped`]); the
//!   caller never waits. A slow disk or a target that hangs slows down
//!   neither the work nor the other sinks.
//! - **Diagnostics, not records.** A dropped line must never lose a result:
//!   what a run found is written by the code that owns it, never by a log.
//!
//! A library crate only uses `tracing`'s macros and never calls [`init`].
//!
//! ```
//! use helpers::logging::{self, Format, Sink, Target};
//! use tracing::Level;
//!
//! let (dispatch, logging) = logging::build(vec![
//!     Sink::new(Target::Stderr, Format::Pretty, Level::WARN),
//! ])?;
//! tracing::dispatcher::with_default(&dispatch, || {
//!     tracing::warn!(ticket = "t1", "the check timed out");
//! });
//! drop(logging); // writes what is still queued
//! # Ok::<(), std::io::Error>(())
//! ```

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use tracing::{Dispatch, Level};
use tracing_appender::non_blocking::{ErrorCounter, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::time::uptime;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{Layer, Registry, fmt};

/// Lines a sink holds while its writer is busy, unless [`Sink::queue`] says
/// otherwise. Further lines are dropped until there is room again.
pub const DEFAULT_QUEUE: usize = 4096;

/// Where a sink's lines go.
pub enum Target {
    /// Standard error. Standard output stays free for a program's result.
    Stderr,
    /// A file, appended to; it and its directory are created when missing.
    File(PathBuf),
    /// Any writer. It runs on the sink's own thread, so it may block, as a
    /// writer does that sends each line over a network.
    Writer(Box<dyn Write + Send>),
}

/// How a sink writes an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// One line for a person: time since the start, level, the open spans,
    /// message and fields. Coloured on a terminal, unless `NO_COLOR` is set.
    Pretty,
    /// One JSON object per line: `timestamp` (UTC), `level`, `target`, the
    /// message and the event's fields at the top level, and the open spans.
    Json,
}

/// One destination of log lines.
pub struct Sink {
    target: Target,
    format: Format,
    level: Level,
    queue: usize,
}

impl Sink {
    /// A sink that takes events of `level` and everything more severe.
    pub fn new(target: Target, format: Format, level: Level) -> Sink {
        Sink {
            target,
            format,
            level,
            queue: DEFAULT_QUEUE,
        }
    }

    /// How many lines wait while the writer is busy (default
    /// [`DEFAULT_QUEUE`], at least one).
    pub fn queue(mut self, lines: usize) -> Sink {
        self.queue = lines.max(1);
        self
    }
}

/// The running sinks. Keep it for as long as the program logs: dropping it
/// writes what is still queued and ends the writer threads. A sink whose
/// writer hangs is given about a second, then left behind.
pub struct Logging {
    guards: Vec<WorkerGuard>,
    counters: Vec<ErrorCounter>,
}

impl Logging {
    /// Lines dropped so far because a sink's queue was full, over all sinks.
    pub fn dropped(&self) -> usize {
        self.counters.iter().map(ErrorCounter::dropped_lines).sum()
    }

    /// How many sinks run.
    pub fn sinks(&self) -> usize {
        self.guards.len()
    }
}

/// Starts the sinks and returns the dispatcher that feeds them, without
/// installing it: for tests, and for a program that scopes its logging with
/// `tracing::dispatcher::with_default`. `Err` when a file cannot be opened.
pub fn build(sinks: Vec<Sink>) -> io::Result<(Dispatch, Logging)> {
    let mut layers = Vec::new();
    let mut logging = Logging {
        guards: Vec::new(),
        counters: Vec::new(),
    };
    for sink in sinks {
        let colour = matches!(sink.target, Target::Stderr)
            && io::stderr().is_terminal()
            && std::env::var_os("NO_COLOR").is_none();
        let target: Box<dyn Write + Send> = match sink.target {
            Target::Stderr => Box::new(io::stderr()),
            Target::File(path) => {
                if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
                    fs::create_dir_all(dir)?;
                }
                Box::new(
                    fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)?,
                )
            }
            Target::Writer(writer) => writer,
        };
        let (writer, guard) = NonBlockingBuilder::default()
            .lossy(true)
            .buffered_lines_limit(sink.queue)
            .thread_name("kai-log")
            .finish(target);
        logging.counters.push(writer.error_counter());
        logging.guards.push(guard);

        let level = LevelFilter::from_level(sink.level);
        let layer: Box<dyn Layer<Registry> + Send + Sync> = match sink.format {
            Format::Pretty => fmt::layer()
                .compact()
                .with_target(false)
                .with_timer(uptime())
                .with_ansi(colour)
                .with_writer(writer)
                .with_filter(level)
                .boxed(),
            Format::Json => fmt::layer()
                .json()
                .flatten_event(true)
                .with_writer(writer)
                .with_filter(level)
                .boxed(),
        };
        layers.push(layer);
    }
    Ok((Dispatch::new(Registry::default().with(layers)), logging))
}

/// Starts the sinks and installs them for the whole program. Call it once,
/// at the start of `main`, and keep the result until `main` returns. `Err`
/// when a file cannot be opened or logging is installed already.
pub fn init(sinks: Vec<Sink>) -> io::Result<Logging> {
    let (dispatch, logging) = build(sinks)?;
    tracing::dispatcher::set_global_default(dispatch).map_err(io::Error::other)?;
    Ok(logging)
}
