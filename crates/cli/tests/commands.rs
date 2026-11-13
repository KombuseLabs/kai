//! `kai ticket list`, `kai ticket qualify` and `kai github sync`, run as a
//! program on a store of invented tickets. No network: the sync tests fail
//! before a request, or at a port on this machine that nothing listens on.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use store::Store;
use types::{Form, IssueId, IssueState, Ticket};

/// A fresh directory of the test's own, outside the repository so that no
/// `.env` or `kai.toml` above it is found.
fn dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kai-commands-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs kai in `dir` with `args` and none of kai's variables from the
/// test's own environment, except those in `env`.
fn kai(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kai"));
    command.current_dir(dir).args(args);
    for (name, _) in std::env::vars() {
        if name.starts_with("KAI_") {
            command.env_remove(name);
        }
    }
    command.envs(env.iter().copied());
    command.output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn ticket(number: u64, labels: &[&str], instruction: &str) -> Ticket {
    Ticket {
        issue: IssueId {
            repo: "example-org/example-board".parse().unwrap(),
            number,
        },
        title: format!("Invented ticket {number}"),
        url: format!("https://github.com/example-org/example-board/issues/{number}"),
        state: IssueState::Open,
        labels: labels.iter().map(|label| label.to_string()).collect(),
        label_events: vec![],
        updated_at: "2026-01-02T03:04:05Z".to_owned(),
        form: Some(Form {
            instruction: Some(instruction.to_owned()),
            repo: Some("example-org/example-code".to_owned()),
            base_ref: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            check: Some("cargo test --locked".to_owned()),
            hidden_tests: Some("tests/hidden/invented.rs".to_owned()),
        }),
    }
}

/// The default store in `dir`, holding three invented tickets: one
/// without the label, one that may be used once the gate is built, and one
/// whose instruction holds a code block.
fn store_with_tickets(dir: &Path) {
    let mut store = Store::open(&dir.join(".kai/store.sqlite")).unwrap();
    let tickets = [
        ticket(1, &[], "Count the widgets."),
        ticket(2, &["kai"], "Count the widgets."),
        ticket(3, &["kai"], "Count them:\n\n```\nwidgets.len()\n```"),
    ];
    let board = "example-org/example-board".parse().unwrap();
    store.record_sync(&board, &tickets, None).unwrap();
}

#[test]
fn list_prints_a_line_per_ticket() {
    let dir = dir("list");
    store_with_tickets(&dir);
    let output = kai(&dir, &["ticket", "list"], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let lines: Vec<String> = stdout(&output).lines().map(str::to_owned).collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[0].starts_with("example-org/example-board#1  open    -  "),
        "{lines:?}"
    );
    assert!(
        lines[1].contains("kai") && lines[1].ends_with("Invented ticket 2"),
        "{lines:?}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn qualify_gives_each_verdict_and_why() {
    let dir = dir("qualify");
    store_with_tickets(&dir);
    let output = kai(&dir, &["ticket", "qualify"], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains("#1  refused  "), "{out}");
    assert!(out.contains("failed       label: "), "{out}");
    assert!(out.contains("#2  undecided"), "{out}");
    assert!(out.contains("not checked  trust-gate: "), "{out}");
    assert!(out.contains("#3  refused  "), "{out}");
    assert!(out.contains("failed       no-code-block: "), "{out}");
    assert!(
        out.ends_with("0 qualified, 2 refused, 1 undecided\n"),
        "{out}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_empty_store_prints_nothing_and_says_how_to_fill_it() {
    let dir = dir("empty");
    let output = kai(&dir, &["ticket", "qualify"], &[]);
    assert!(output.status.success());
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("kai github sync"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn store_reset_empties_the_store() {
    let dir = dir("reset");
    store_with_tickets(&dir);
    let output = kai(&dir, &["store", "reset"], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).ends_with(": emptied, 3 tickets removed\n"),
        "{}",
        stdout(&output)
    );
    let store = Store::open(&dir.join(".kai/store.sqlite")).unwrap();
    assert!(store.tickets().unwrap().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn store_reset_without_a_store_makes_none() {
    let dir = dir("reset-none");
    let output = kai(&dir, &["store", "reset"], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("no store here"),
        "{}",
        stdout(&output)
    );
    assert!(!dir.join(".kai").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

/// Whether another program's database is refused is the store's test;
/// here, that `reset` goes through the same check and removes no file.
#[test]
fn store_reset_leaves_a_file_that_is_not_kais_store_alone() {
    let dir = dir("reset-foreign");
    let text = "not a database, but long enough to have a header of its own.\n".repeat(3);
    std::fs::write(dir.join("other.sqlite"), &text).unwrap();
    let output = kai(&dir, &["store", "reset", "--store", "other.sqlite"], &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("cannot open the store"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("other.sqlite")).unwrap(),
        text
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sync_without_a_board_is_a_usage_error() {
    let dir = dir("no-board");
    let output = kai(&dir, &["github", "sync"], &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("no board to sync"),
        "{}",
        stderr(&output)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_board_in_kai_toml_that_is_not_owner_slash_name_is_a_usage_error() {
    let dir = dir("bad-board");
    std::fs::write(
        dir.join("kai.toml"),
        "[github]\nboard = \"example-board\"\n",
    )
    .unwrap();
    let output = kai(&dir, &["github", "sync"], &[]);
    assert_eq!(output.status.code(), Some(2));
    let err = stderr(&output);
    assert!(
        err.contains("github.board in ") && err.contains("owner/name"),
        "{err}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sync_that_cannot_reach_github_fails_and_leaves_the_store_as_it_was() {
    let dir = dir("unreachable");
    store_with_tickets(&dir);
    // Port 9 (discard) on this machine: nothing listens, nothing leaves it.
    std::fs::write(
        dir.join("kai.toml"),
        "[github]\napi = \"http://127.0.0.1:9\"\n",
    )
    .unwrap();
    let env = [
        ("KAI_GITHUB_TOKEN", "invented-token-0123456789"),
        ("KAI_GITHUB_BOARD", "example-org/example-board"),
    ];
    let output = kai(&dir, &["github", "sync"], &env);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(
        err.contains("cannot fetch the issues of example-org/example-board"),
        "{err}"
    );
    assert!(!err.contains("invented-token"), "{err}");
    let store = Store::open(&dir.join(".kai/store.sqlite")).unwrap();
    assert_eq!(store.tickets().unwrap().len(), 3);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn deps_refuses_a_ticket_its_rules_refuse() {
    let dir = dir("deps-refused");
    store_with_tickets(&dir);
    let cache = dir.join("cache");
    let output = kai(
        &dir,
        &["ticket", "deps", "1", "--cache", cache.to_str().unwrap()],
        &[],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        stderr(&output).contains("is refused: label: "),
        "{}",
        stderr(&output)
    );
    assert!(!cache.exists(), "nothing is made for a refused ticket");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn deps_needs_the_image_of_the_rust_recipe() {
    let dir = dir("deps-image");
    store_with_tickets(&dir);
    let cache = dir.join("cache");
    let output = kai(
        &dir,
        &["ticket", "deps", "2", "--cache", cache.to_str().unwrap()],
        &[],
    );
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        stderr(&output).contains("`kai vm image build rust`"),
        "{}",
        stderr(&output)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn deps_takes_a_repository_and_a_full_sha_instead_of_a_ticket() {
    let dir = dir("deps-repo");
    let output = kai(
        &dir,
        &[
            "ticket",
            "deps",
            "--repo",
            "example-org/example-code",
            "--ref",
            "abc123",
        ],
        &[],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(stderr(&output).contains("full sha"), "{}", stderr(&output));
    let output = kai(
        &dir,
        &["ticket", "deps", "--repo", "example-org/example-code"],
        &[],
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "a repository without a ref: {output:?}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
