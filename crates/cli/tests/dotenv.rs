//! The `.env` file: found from the current directory upward, beaten by a
//! variable that is already set, and never quoted when it is malformed.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory with a `.env` holding `contents` and an empty
/// subdirectory `sub`, where kai is run from.
fn dir_with_dotenv(test: &str, contents: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kai-dotenv-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join(".env"), contents).unwrap();
    dir
}

/// Runs kai in `dir/sub` with none of kai's variables from the test's own
/// environment.
fn kai(dir: &Path, env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kai"));
    command.current_dir(dir.join("sub"));
    for (name, _) in std::env::vars() {
        if name.starts_with("KAI_") {
            command.env_remove(name);
        }
    }
    command.envs(env.iter().copied());
    command.output().unwrap()
}

#[test]
fn a_dotenv_in_a_parent_directory_sets_the_variables() {
    let dir = dir_with_dotenv("parent", "KAI_LOG_LEVEL=debug\n");
    let output = kai(&dir, &[]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("kai started"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_variable_already_set_wins_over_the_dotenv() {
    let dir = dir_with_dotenv("set", "KAI_LOG_LEVEL=debug\n");
    let output = kai(&dir, &[("KAI_LOG_LEVEL", "off")]);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_malformed_line_is_refused_without_quoting_it() {
    let secret = "invented-secret-1234";
    let dir = dir_with_dotenv("malformed", &format!("KAI_GITHUB_TOKEN {secret} \"\n"));
    let output = kai(&dir, &[]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot read .env"), "{stderr}");
    assert!(!stderr.contains(secret), "{stderr}");
    std::fs::remove_dir_all(dir).unwrap();
}
