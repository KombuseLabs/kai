//! Prints the Rust stubs of a planned class diagram.
//!
//! ```text
//! diagram-stubs <file.md>[:<line>]
//! diagram-stubs <file.mmd>
//! ```
//!
//! In a Markdown file it takes the Mermaid block whose fence, or first
//! line, is at `<line>`; without a line, the file's only class diagram. A
//! `.mmd` file is one diagram. It prints the stubs to standard output,
//! formatted by rustfmt, and on standard error the types the diagram uses
//! but does not show. When it cannot make stubs it prints nothing to
//! standard output, `file:line: reason` for each reason to standard error,
//! and exits with 1. It writes no file.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a maintainer's tool: its result is what it prints, and it runs no logger"
)]

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use diagrams::markdown::{Block, mermaid_blocks};
use diagrams::mermaid::{Kind, class};

const USAGE: &str = "usage: diagram-stubs <file.md>[:<line>] | <file.mmd>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [arg] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match run(arg) {
        Ok(()) => ExitCode::SUCCESS,
        Err(lines) => {
            for line in lines {
                eprintln!("{line}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(arg: &str) -> Result<(), Vec<String>> {
    let (file, line) = match arg.rsplit_once(':') {
        Some((file, line)) if !line.is_empty() && line.chars().all(|c| c.is_ascii_digit()) => (
            file,
            Some(
                line.parse::<usize>()
                    .map_err(|e| vec![format!("{arg}: {e}")])?,
            ),
        ),
        _ => (arg, None),
    };
    let text = std::fs::read_to_string(file).map_err(|err| vec![format!("{file}: {err}")])?;
    let block = pick(file, &text, line).map_err(|why| vec![why])?;
    let diagram = class::parse(&block.text).map_err(|err| {
        vec![format!(
            "{file}:{}: {}",
            block.file_line(err.line),
            err.message
        )]
    })?;
    let stubs = diagrams::stubs::generate(&diagram).map_err(|problems| {
        problems
            .iter()
            .map(|problem| {
                format!(
                    "{file}:{}: {}",
                    block.file_line(problem.line),
                    problem.message
                )
            })
            .collect::<Vec<_>>()
    })?;
    let formatted = rustfmt(&stubs.code).map_err(|why| vec![why])?;
    print!("{formatted}");
    if !stubs.not_shown.is_empty() {
        eprintln!(
            "not shown (define or import them): {}",
            stubs.not_shown.join(", ")
        );
    }
    Ok(())
}

/// The block to make stubs of.
fn pick(file: &str, text: &str, line: Option<usize>) -> Result<Block, String> {
    if Path::new(file).extension().is_some_and(|ext| ext == "mmd") {
        return Ok(Block {
            fence_line: 0,
            first_line: 1,
            text: text.to_owned(),
            unclosed: false,
        });
    }
    let blocks = mermaid_blocks(text);
    match line {
        Some(line) => blocks
            .into_iter()
            .find(|block| block.fence_line == line || block.first_line == line)
            .ok_or_else(|| format!("{file}:{line}: no Mermaid block starts here")),
        None => {
            let mut classes: Vec<Block> = blocks
                .into_iter()
                .filter(|block| Kind::of(&block.text).0 == Kind::Class)
                .collect();
            match classes.len() {
                1 => Ok(classes.remove(0)),
                0 => Err(format!("{file}: no class diagram")),
                _ => {
                    let lines: Vec<String> = classes
                        .iter()
                        .map(|block| block.fence_line.to_string())
                        .collect();
                    Err(format!(
                        "{file}: {} class diagrams, at lines {}; name one as {file}:<line>",
                        classes.len(),
                        lines.join(", ")
                    ))
                }
            }
        }
    }
}

/// `code` formatted by rustfmt, read from standard input.
fn rustfmt(code: &str) -> Result<String, String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("cannot run rustfmt: {err}"))?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(code.as_bytes())
        .map_err(|err| format!("cannot give rustfmt the stubs: {err}"))?;
    let output = child
        .wait_with_output()
        .map_err(|err| format!("rustfmt did not finish: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "rustfmt refused the stubs ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|err| format!("rustfmt wrote no UTF-8: {err}"))
}
