//! `kai study estimate`, run as a program on invented study files. No
//! network, no model.

use helpers::testing::TestDir;
use std::path::Path;
use std::process::{Command, Output};

/// A fresh directory of the test's own, outside the repository so that no
/// `.env` or `kai.toml` above it is found.
fn dir(test: &str) -> TestDir {
    TestDir::new(&format!("cli-study-{test}"))
}

fn kai(dir: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kai"));
    command.current_dir(dir).args(args);
    for (name, _) in std::env::vars() {
        if name.starts_with("KAI_") {
            command.env_remove(name);
        }
    }
    command.output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const STUDY: &str = r#"
tasks = 24
arms = ["shadow", "ghost"]
repetitions = 2
max_turns = 40
context_tokens = 200000
name = "a key the estimate does not read"
"#;

#[test]
fn estimate_prints_what_the_study_can_see() {
    let dir = dir("estimate");
    std::fs::write(dir.join("study.toml"), STUDY).unwrap();
    let out = kai(&dir, &["study", "estimate", "study.toml"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("trials: 96 (24 tasks x 2 repetitions x 2 arms)"),
        "{text}"
    );
    assert!(text.contains("spread per task: 0.381 (assumed"), "{text}");
    assert!(text.contains("detectable saving: 20.3 %"), "{text}");
    assert!(text.contains("true saving  20 %:  50.0 %"), "{text}");
    assert!(text.contains("890880000 in all"), "{text}");
}

#[test]
fn the_thresholds_and_an_observed_spread_are_parameters() {
    let dir = dir("params");
    let file = format!("{STUDY}\n[question]\nmin_saving = 0.10\nmargin = 0.2\nalpha = 0.10\n");
    std::fs::write(dir.join("study.toml"), file).unwrap();
    let out = kai(
        &dir,
        &["study", "estimate", "study.toml", "--observed-sd", "0.2"],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("spread per task: 0.200 (observed in a pilot)"),
        "{text}"
    );
    assert!(text.contains("saving of at least 10 %"), "{text}");
    assert!(text.contains("alpha 0.1"), "{text}");
}

#[test]
fn a_file_that_cannot_be_used_is_a_usage_error() {
    let dir = dir("bad");
    let out = kai(&dir, &["study", "estimate", "missing.toml"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("cannot read"), "{}", stderr(&out));
    // A study of one task has no interval.
    std::fs::write(
        dir.join("one.toml"),
        "tasks = 1\narms = 2\nrepetitions = 2\nmax_turns = 40\ncontext_tokens = 200000\n",
    )
    .unwrap();
    let out = kai(&dir, &["study", "estimate", "one.toml"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("at least 2 tasks"),
        "{}",
        stderr(&out)
    );
    // A key the estimate needs is missing.
    std::fs::write(dir.join("part.toml"), "tasks = 5\n").unwrap();
    let out = kai(&dir, &["study", "estimate", "part.toml"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("missing field"), "{}", stderr(&out));
    assert!(stdout(&out).is_empty());
}
