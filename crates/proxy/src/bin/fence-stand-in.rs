//! A stand-in for `fence proxy`, for the tests of the `proxy` crate only.
//!
//! It keeps the contract `Egress` relies on and nothing more: it listens on
//! the socket `--listen=unix:<path>` names (relative to its working
//! directory), says `listening unix:<path>` on standard output, answers
//! each `CONNECT host:port` with `200` when a `--net=<host:port>` names it
//! (or in `--learn` mode) and `403` otherwise, without connecting anywhere,
//! writes one ledger line per decision in fence's format, and ends with 0
//! once its standard input closes.
//!
//! What it was given goes to `stand-in.args`, and the names of its
//! environment to `stand-in.env`, in its working directory. A file
//! `stand-in.mode` there makes it break the contract on purpose, one way
//! per [`Misbehaviour`].

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// How long a misbehaving stand-in hangs; the tests kill it long before.
const HANG: Duration = Duration::from_secs(60);

/// The largest request head it reads.
const HEAD_MAX: u64 = 64 * 1024;

fn main() -> ExitCode {
    let given: Vec<String> = std::env::args().skip(1).collect();
    record(&given);
    let misbehaviour = match Misbehaviour::read() {
        Ok(misbehaviour) => misbehaviour,
        Err(why) => return exit(2, &why),
    };
    let args = match Args::parse(&given) {
        Ok(args) => args,
        Err(why) => return exit(2, &why),
    };
    match misbehaviour {
        Misbehaviour::Refuse => return exit(2, "--net nonsense is not host:port"),
        Misbehaviour::Silent => {
            thread::sleep(HANG);
            return ExitCode::SUCCESS;
        }
        _ => {}
    }

    let (listener, ledger) = match open(&args) {
        Ok(opened) => opened,
        Err(why) => return exit(1, &why),
    };
    if misbehaviour == Misbehaviour::BadLedger {
        ledger.write_raw("this is not json");
    }
    say_ready(&args.listen, misbehaviour == Misbehaviour::Chatty);
    serve(listener, Arc::new(ledger), Arc::new(args.net), args.learn);

    // The contract's end: standard input closes.
    let _ = io::copy(&mut io::stdin(), &mut io::sink());
    if misbehaviour == Misbehaviour::Stubborn {
        thread::sleep(HANG);
    }
    let _ = fs::remove_file(&args.listen);
    if misbehaviour == Misbehaviour::Failing {
        return exit(3, "failing on purpose");
    }
    ExitCode::SUCCESS
}

/// How the stand-in breaks the contract when a test asks it to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Misbehaviour {
    /// None: it keeps the contract.
    None,
    /// `refuse`: refuses its command line, exit 2 before listening.
    Refuse,
    /// `silent`: never says it listens.
    Silent,
    /// `chatty`: writes another line before the ready line.
    Chatty,
    /// `stubborn`: does not end when its input closes.
    Stubborn,
    /// `bad-ledger`: writes a ledger line that is not JSON.
    BadLedger,
    /// `failing`: ends with 3 once its input closes.
    Failing,
}

impl Misbehaviour {
    /// Reads `stand-in.mode` in the working directory; no file is none. A
    /// word it does not know is an error, so that a test's typo shows.
    fn read() -> Result<Misbehaviour, String> {
        let mode = fs::read_to_string("stand-in.mode").unwrap_or_default();
        Ok(match mode.trim() {
            "" => Misbehaviour::None,
            "refuse" => Misbehaviour::Refuse,
            "silent" => Misbehaviour::Silent,
            "chatty" => Misbehaviour::Chatty,
            "stubborn" => Misbehaviour::Stubborn,
            "bad-ledger" => Misbehaviour::BadLedger,
            "failing" => Misbehaviour::Failing,
            other => return Err(format!("stand-in.mode {other:?} is no misbehaviour")),
        })
    }
}

/// What `fence proxy` is started with, in the contract's form:
/// `proxy --listen=unix:<path> [--net=<rule>]… --ledger=<file> [--learn]`.
struct Args {
    listen: String,
    net: Vec<String>,
    ledger: String,
    learn: bool,
}

impl Args {
    fn parse(given: &[String]) -> Result<Args, String> {
        let Some(("proxy", rest)) = given.split_first().map(|(c, r)| (c.as_str(), r)) else {
            return Err("the first argument is not proxy".to_owned());
        };
        let (mut listen, mut ledger) = (None, None);
        let mut args = Args {
            listen: String::new(),
            net: Vec::new(),
            ledger: String::new(),
            learn: false,
        };
        for arg in rest {
            if let Some(path) = arg.strip_prefix("--listen=unix:") {
                listen = Some(path.to_owned());
            } else if let Some(rule) = arg.strip_prefix("--net=") {
                args.net.push(rule.to_ascii_lowercase());
            } else if let Some(file) = arg.strip_prefix("--ledger=") {
                ledger = Some(file.to_owned());
            } else if arg == "--learn" {
                args.learn = true;
            } else {
                return Err(format!("unknown argument {arg}"));
            }
        }
        let (Some(listen), Some(ledger)) = (listen, ledger) else {
            return Err("--listen and --ledger are required".to_owned());
        };
        args.listen = listen;
        args.ledger = ledger;
        Ok(args)
    }
}

/// Writes what it was given into its working directory, for the tests to
/// read.
fn record(given: &[String]) {
    let names: Vec<String> = std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    let _ = fs::write("stand-in.args", given.join("\n"));
    let _ = fs::write("stand-in.env", names.join("\n"));
}

/// The ledger, written by every connection's thread.
struct Ledger(Mutex<File>);

impl Ledger {
    /// One decision, in fence's format.
    fn write(&self, host: &str, port: u16, decision: &str) {
        let line = serde_json::json!({
            "v": 1,
            "ts": "2026-11-13T00:00:00Z",
            "kind": "net",
            "host": host,
            "port": port,
            "via": "connect",
            "decision": decision,
        });
        self.write_raw(&line.to_string());
    }

    fn write_raw(&self, line: &str) {
        if let Ok(mut file) = self.0.lock() {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// Makes the ledger and the socket, as fence does: neither may be there
/// already.
fn open(args: &Args) -> Result<(UnixListener, Ledger), String> {
    let file = OpenOptions::new()
        .append(true)
        .create_new(true)
        .open(&args.ledger)
        .map_err(|e| format!("cannot open the ledger {}: {e}", args.ledger))?;
    let listener = UnixListener::bind(&args.listen)
        .map_err(|e| format!("cannot listen on {}: {e}", args.listen))?;
    Ok((listener, Ledger(Mutex::new(file))))
}

/// The contract's one line on standard output, or another line first.
fn say_ready(listen: &str, chatty: bool) {
    let mut out = io::stdout();
    if chatty {
        let _ = writeln!(out, "hello");
    }
    let _ = writeln!(out, "listening unix:{listen}");
    let _ = out.flush();
}

/// Answers every connection on a thread of its own, until the process
/// ends.
fn serve(listener: UnixListener, ledger: Arc<Ledger>, net: Arc<Vec<String>>, learn: bool) {
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (ledger, net) = (ledger.clone(), net.clone());
            thread::spawn(move || answer(&stream, &ledger, &net, learn));
        }
    });
}

/// Reads one request, decides, records, answers. It connects nowhere: an
/// allowed `CONNECT` gets its `200` and nothing more.
fn answer(stream: &UnixStream, ledger: &Ledger, net: &[String], learn: bool) {
    let Some(request) = request_line(stream) else {
        return;
    };
    let Some((host, port)) = connect_target(&request) else {
        let _ =
            (&*stream).write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n");
        return;
    };
    let shown = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let decision = match (net.contains(&shown), learn) {
        (true, _) => "allow",
        (false, true) => "would-deny",
        (false, false) => "deny",
    };
    // Recorded before it is answered, so that a client that has its answer
    // finds the line.
    ledger.write(&host, port, decision);
    if decision == "deny" {
        let _ = writeln!(io::stderr(), "fence: refused {shown}: not in net.allow");
        let _ = (&*stream).write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
    } else {
        let _ = (&*stream).write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n");
    }
}

/// Reads a request's head up to its blank line and returns its first line.
fn request_line(stream: &UnixStream) -> Option<String> {
    let mut reader = BufReader::new(stream.take(HEAD_MAX));
    let mut first = String::new();
    reader.read_line(&mut first).ok()?;
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
        line.clear();
    }
    Some(first)
}

/// `CONNECT host:port HTTP/1.1` to its host, in lower case and without
/// brackets, and its port.
fn connect_target(request: &str) -> Option<(String, u16)> {
    let mut parts = request.split_whitespace();
    let ("CONNECT", target) = (parts.next()?, parts.next()?) else {
        return None;
    };
    let (host, port) = target.rsplit_once(':')?;
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    Some((host, port.parse().ok()?))
}

/// Says why on standard error, as fence does, and ends with `code`.
fn exit(code: u8, why: &str) -> ExitCode {
    let _ = writeln!(io::stderr(), "fence: {why}");
    ExitCode::from(code)
}
