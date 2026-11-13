//! A program for the launcher's tests, which put it into a test image as
//! `/probe`. It is built with the init, statically, so that a test image
//! needs nothing else.
//!
//! - `probe exit <n>` writes `out` to standard output and `err` to standard
//!   error, and exits with `n`;
//! - `probe sleep` writes `before the hang` and sleeps until it is killed;
//! - `probe do <step> [; <step>]…` does each step and exits with 0, where a
//!   step is `write <path> <text>`, `remove <path>` (a file or a whole
//!   directory), `cat <path>` (its text, or `missing <path>`), `ls <dir>`
//!   (the names in it, sorted, one line), `reset-dir <dir>` (emptied, then
//!   holding `new`), `id` (the user and group ids, as `uid:gid`), `owner
//!   <path>` (the owner of the path itself, a link not followed, and its
//!   mode, as `uid:gid <octal>`), `can-write <file>` (whether the file
//!   opens for writing, as `yes <file>` or `no <file>`; nothing is
//!   written), `can-create <dir>` (whether a new name can be made in the
//!   directory, which also decides whether one can be removed or replaced
//!   there; what is made is removed) or
//!   `connect <port> <text>` (sends the text, `\r\n` taken as a line end, to
//!   `127.0.0.1:<port>` and prints the first line of the answer, or
//!   `cannot connect`).

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "writing to both streams is what the probe is for"
)]

use std::io::Write as _;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["exit", code] => {
            println!("out");
            eprintln!("err");
            std::process::exit(code.parse().unwrap_or(2));
        }
        ["sleep"] => {
            println!("before the hang");
            let _ = std::io::stdout().flush();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
        ["do", steps @ ..] => {
            for step in steps.split(|word| *word == ";") {
                if let Err(err) = run(step) {
                    eprintln!("probe: {step:?}: {err}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: probe exit <n> | probe sleep | probe do <step> [; <step>]...");
            std::process::exit(2);
        }
    }
}

fn run(step: &[&str]) -> std::io::Result<()> {
    match step {
        ["write", path, text] => {
            if let Some(parent) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, text)
        }
        ["remove", path] => {
            if std::fs::symlink_metadata(path)?.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            }
        }
        ["cat", path] => {
            match std::fs::read_to_string(path) {
                Ok(text) => println!("{text}"),
                Err(_) => println!("missing {path}"),
            }
            Ok(())
        }
        ["ls", dir] => {
            let mut names: Vec<String> = std::fs::read_dir(dir)?
                .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
                .collect::<Result<_, _>>()?;
            names.sort();
            println!("{}", names.join(" "));
            Ok(())
        }
        ["reset-dir", dir] => {
            std::fs::remove_dir_all(dir)?;
            std::fs::create_dir(dir)?;
            std::fs::write(format!("{dir}/new"), "new")
        }
        ["id"] => {
            let status = std::fs::read_to_string("/proc/self/status")?;
            let field = |name: &str| {
                status
                    .lines()
                    .find_map(|line| line.strip_prefix(name))
                    .and_then(|rest| rest.split_whitespace().next())
                    .unwrap_or("?")
                    .to_owned()
            };
            println!("{}:{}", field("Uid:"), field("Gid:"));
            Ok(())
        }
        ["owner", path] => {
            use std::os::unix::fs::MetadataExt as _;
            let meta = std::fs::symlink_metadata(path)?;
            println!("{}:{} {:o}", meta.uid(), meta.gid(), meta.mode() & 0o7777);
            Ok(())
        }
        ["can-write", path] => {
            let opened = std::fs::OpenOptions::new().write(true).open(path);
            println!("{} {path}", if opened.is_ok() { "yes" } else { "no" });
            Ok(())
        }
        ["can-create", dir] => {
            let probe = format!("{dir}/.probe-can-create");
            let made = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&probe);
            if made.is_ok() {
                std::fs::remove_file(&probe)?;
            }
            println!("{} {dir}", if made.is_ok() { "yes" } else { "no" });
            Ok(())
        }
        ["connect", port, text] => {
            use std::io::{BufRead as _, Write as _};
            let Ok(mut stream) =
                std::net::TcpStream::connect(("127.0.0.1", port.parse().unwrap_or(0)))
            else {
                println!("cannot connect");
                return Ok(());
            };
            stream.write_all(text.replace("\\r\\n", "\r\n").as_bytes())?;
            let mut line = String::new();
            std::io::BufReader::new(stream).read_line(&mut line)?;
            println!("{}", line.trim_end());
            Ok(())
        }
        _ => Err(std::io::Error::other("unknown step")),
    }
}
