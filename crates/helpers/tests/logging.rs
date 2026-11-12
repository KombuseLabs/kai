//! `helpers::logging`: what each sink receives, in which format, and that
//! logging never waits for a writer.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use helpers::logging::{self, Format, Sink, Target};
use tracing::Level;

/// A writer whose bytes a test can read afterwards.
#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A writer that does not return from `write` until the test lets it.
struct Stuck(mpsc::Receiver<()>);

impl Write for Stuck {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let _ = self.0.recv();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn file(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("logging-{name}/sub/kai.jsonl"));
    let _ = fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    path
}

#[test]
fn a_json_file_holds_one_object_per_event_with_its_fields() {
    let path = file("json");
    let (dispatch, logging) = logging::build(vec![Sink::new(
        Target::File(path.clone()),
        Format::Json,
        Level::INFO,
    )])
    .unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        let span = tracing::info_span!("trial", variant = "none");
        let _in_span = span.enter();
        tracing::info!(ticket = "t1", cost_usd = 0.5, "trial finished");
        tracing::debug!("below the sink's level");
        tracing::error!(ticket = "t2", "the check timed out");
    });
    drop(logging);

    let text = fs::read_to_string(&path).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert_eq!(lines[0]["level"], "INFO");
    assert_eq!(lines[0]["message"], "trial finished");
    assert_eq!(lines[0]["ticket"], "t1");
    assert_eq!(lines[0]["cost_usd"], 0.5);
    assert_eq!(lines[0]["span"]["variant"], "none");
    assert!(lines[0]["timestamp"].as_str().unwrap().ends_with('Z'));
    assert_eq!(lines[1]["level"], "ERROR");
    assert_eq!(lines[1]["ticket"], "t2");
}

#[test]
fn each_sink_has_its_own_level_and_format() {
    let (people, machines) = (Buffer::default(), Buffer::default());
    let (dispatch, logging) = logging::build(vec![
        Sink::new(
            Target::Writer(Box::new(people.clone())),
            Format::Pretty,
            Level::WARN,
        ),
        Sink::new(
            Target::Writer(Box::new(machines.clone())),
            Format::Json,
            Level::DEBUG,
        ),
    ])
    .unwrap();
    assert_eq!(logging.sinks(), 2);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::debug!("a detail");
        tracing::warn!(ticket = "t1", "slow check");
    });
    drop(logging);

    let people = people.text();
    assert_eq!(people.lines().count(), 1, "{people}");
    assert!(people.contains("WARN"), "{people}");
    assert!(people.contains("slow check"), "{people}");
    assert!(people.contains("ticket=\"t1\""), "{people}");
    // Colour is for a terminal only.
    assert!(!people.contains('\u{1b}'), "{people:?}");

    let machines = machines.text();
    assert_eq!(machines.lines().count(), 2, "{machines}");
    assert!(machines.contains("a detail"), "{machines}");
}

#[test]
fn a_writer_that_hangs_holds_up_neither_the_caller_nor_another_sink() {
    let (release, stuck) = mpsc::channel();
    let healthy = Buffer::default();
    let (dispatch, logging) = logging::build(vec![
        Sink::new(
            Target::Writer(Box::new(Stuck(stuck))),
            Format::Json,
            Level::INFO,
        )
        .queue(4),
        Sink::new(
            Target::Writer(Box::new(healthy.clone())),
            Format::Json,
            Level::INFO,
        ),
    ])
    .unwrap();

    // The caller logs on a thread of its own, so that a logger that waited
    // for the stuck writer fails this test instead of hanging it.
    let (done, finished) = mpsc::channel();
    let caller = thread::spawn(move || {
        tracing::dispatcher::with_default(&dispatch, || {
            for n in 0..100 {
                tracing::info!(n, "event");
            }
        });
        done.send(()).unwrap();
    });
    finished
        .recv_timeout(Duration::from_secs(10))
        .expect("logging waited for a writer that hangs");
    caller.join().unwrap();

    // The stuck sink kept what its queue holds and dropped the rest.
    assert!(logging.dropped() >= 90, "dropped {}", logging.dropped());

    drop(release); // the stuck writer returns, so the sinks can end
    drop(logging);
    assert_eq!(healthy.text().lines().count(), 100);
}

#[test]
fn a_file_that_cannot_be_opened_is_an_error() {
    let path = file("unopenable");
    fs::create_dir_all(&path).unwrap(); // a directory where the file should be
    let result = logging::build(vec![Sink::new(
        Target::File(path),
        Format::Json,
        Level::INFO,
    )]);
    assert!(result.is_err());
}
