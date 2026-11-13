//! `kai vm doctor`, `kai vm run` up to where it would need a VM, and where
//! `kai vm` keeps its cache, run as a program. No network and no VM:
//! nothing here fetches, and the runs that boot are crates/vm's tests.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory of the test's own, outside the repository so that no
/// `.env` or `kai.toml` above it is found.
fn dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kai-vm-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs kai in `dir` with `args` and none of kai's variables from the
/// test's own environment.
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

#[test]
fn doctor_with_an_empty_cache_fails_and_names_kai_vm_fetch() {
    let dir = dir("empty");
    let cache = dir.join("cache");
    let output = kai(&dir, &["vm", "doctor", "--cache", cache.to_str().unwrap()]);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stdout = stdout(&output);
    for name in ["Firecracker v1.18.0", "guest kernel 6.1.155"] {
        let line = stdout
            .lines()
            .find(|line| line.contains(name))
            .unwrap_or_else(|| panic!("no line for {name}: {stdout}"));
        assert!(line.starts_with("missing"), "{line}");
        assert!(line.contains("`kai vm fetch` downloads it"), "{line}");
    }
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("needs are not met"),
        "{output:?}"
    );
    // Looking makes nothing.
    assert!(!cache.exists());
}

#[test]
fn the_cache_in_kai_toml_is_taken_from_the_files_directory() {
    let dir = dir("config");
    std::fs::write(dir.join("kai.toml"), "[vm]\ncache = \"vm-cache\"\n").unwrap();
    std::fs::create_dir(dir.join("below")).unwrap();
    let output = kai(&dir.join("below"), &["vm", "doctor"]);

    let expected = format!("not in {}", dir.join("vm-cache").display());
    assert!(stdout(&output).contains(&expected), "{output:?}");
}

#[test]
fn the_cache_option_wins_over_kai_toml() {
    let dir = dir("option");
    std::fs::write(dir.join("kai.toml"), "[vm]\ncache = \"vm-cache\"\n").unwrap();
    let output = kai(&dir, &["vm", "doctor", "--cache", "elsewhere"]);

    assert!(stdout(&output).contains("not in elsewhere;"), "{output:?}");
}

#[test]
fn run_without_the_artefacts_exits_125_and_names_kai_vm_fetch() {
    let dir = dir("run-empty");
    std::fs::write(dir.join("image.sqfs"), b"not used").unwrap();
    let cache = dir.join("cache");
    let output = kai(
        &dir,
        &[
            "vm",
            "run",
            "--cache",
            cache.to_str().unwrap(),
            "--image",
            "image.sqfs",
            "--",
            "/probe",
        ],
    );

    assert_eq!(output.status.code(), Some(125), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("`kai vm fetch`"), "{stderr}");
    assert!(
        !cache.join("runs").exists(),
        "no run directory without a run"
    );
}

#[test]
fn run_refuses_an_environment_entry_that_is_not_name_equals_value_and_never_shows_it() {
    let dir = dir("run-env");
    // An entry may hold a secret, a token pasted without its name above
    // all: none of it reaches standard error.
    for entry in [
        "invented-secret-without-a-name",
        "1X=invented-secret",
        "A-B=invented-secret",
    ] {
        let output = kai(
            &dir,
            &[
                "vm", "run", "--image", "x", "--env", "OK=1", "--env", entry, "--", "true",
            ],
        );
        assert_eq!(output.status.code(), Some(2), "{entry}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("--env #2"), "{entry}: {stderr}");
        assert!(!stderr.contains("invented-secret"), "{entry}: {stderr}");
        assert!(
            !stderr.contains("1X") && !stderr.contains("A-B"),
            "{entry}: {stderr}"
        );
    }
}

#[test]
fn run_refuses_a_vm_without_processors_or_with_too_little_memory() {
    let dir = dir("run-size");
    for (option, value, range) in [("--cpus", "0", "1..=32"), ("--memory", "64", "128..")] {
        let output = kai(
            &dir,
            &["vm", "run", "--image", "x", option, value, "--", "true"],
        );
        assert_eq!(output.status.code(), Some(2), "{option}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(range),
            "{option}: {output:?}"
        );
    }
}

#[test]
fn run_on_a_recipe_that_is_not_built_names_the_command_that_builds_it() {
    let dir = dir("run-recipe");
    let cache = dir.join("cache");
    let output = kai(
        &dir,
        &[
            "vm",
            "run",
            "--cache",
            cache.to_str().unwrap(),
            "--image",
            "rust",
            "--",
            "true",
        ],
    );

    assert_eq!(output.status.code(), Some(125), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("`kai vm image build rust`"), "{stderr}");
}

#[test]
fn image_list_names_every_recipe_and_whether_it_is_built() {
    let dir = dir("image-list");
    let cache = dir.join("cache");
    let output = kai(
        &dir,
        &["vm", "image", "list", "--cache", cache.to_str().unwrap()],
    );

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = stdout(&output);
    assert!(stdout.starts_with("rust  sha256:"), "{stdout}");
    assert!(
        stdout.contains("not built; `kai vm image build rust` builds it"),
        "{stdout}"
    );
}

#[test]
fn image_build_of_a_recipe_kai_does_not_have_is_refused() {
    let dir = dir("image-unknown");
    let output = kai(&dir, &["vm", "image", "build", "python"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("kai has no recipe `python`"),
        "{output:?}"
    );
}

#[test]
fn run_refuses_a_file_or_user_it_cannot_read() {
    let dir = dir("run-parse");
    for args in [
        ["--file", "no-equals-sign"],
        ["--file", "=hidden/x"],
        ["--user", "1000"],
        ["--user", "a:b"],
    ] {
        let mut argv = vec!["vm", "run", "--image", "x"];
        argv.extend(args);
        argv.extend(["--", "true"]);
        let output = kai(&dir, &argv);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    }
}
