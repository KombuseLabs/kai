//! `helpers::config`: which file is found, what is read from it, and what
//! an error says.

use std::error::Error as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use helpers::config::{self, Loaded};
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Settings {
    seed: Option<u64>,
    log: Option<PathBuf>,
}

/// An empty directory of the test's own. The names the tests look for are
/// their own too, so that no file above the target directory is found.
fn dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("config-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_nearest_file_is_found() {
    let dir = dir("nearest");
    let deep = dir.join("a/b/c");
    fs::create_dir_all(&deep).unwrap();
    fs::write(dir.join("nearest.toml"), "").unwrap();
    fs::write(dir.join("a/nearest.toml"), "").unwrap();

    assert_eq!(
        config::find(&deep, "nearest.toml"),
        Some(dir.join("a/nearest.toml"))
    );
    assert_eq!(
        config::find(&dir.join("a"), "nearest.toml"),
        Some(dir.join("a/nearest.toml"))
    );
    assert_eq!(
        config::find(&dir, "nearest.toml"),
        Some(dir.join("nearest.toml"))
    );
}

#[test]
fn nothing_is_found_where_there_is_no_file_of_that_name() {
    let dir = dir("absent");
    assert_eq!(config::find(&dir, "absent-c7f1e2.toml"), None);
}

#[test]
fn a_directory_of_that_name_is_passed_over() {
    let dir = dir("directory");
    fs::create_dir_all(dir.join("sub/directory.toml")).unwrap();
    fs::write(dir.join("directory.toml"), "").unwrap();

    assert_eq!(
        config::find(&dir.join("sub"), "directory.toml"),
        Some(dir.join("directory.toml"))
    );
}

#[test]
fn a_file_is_read_and_its_paths_are_taken_from_its_directory() {
    let dir = dir("read");
    let path = dir.join("read.toml");
    fs::write(&path, "seed = 7\nlog = \"logs/run.jsonl\"\n").unwrap();

    let loaded: Loaded<Settings> = config::load(&path).unwrap();
    assert_eq!(
        loaded.value,
        Settings {
            seed: Some(7),
            log: Some(PathBuf::from("logs/run.jsonl")),
        }
    );
    assert_eq!(loaded.path, path);
    assert_eq!(
        loaded.resolve(Path::new("logs/run.jsonl")),
        dir.join("logs/run.jsonl")
    );
    assert_eq!(
        loaded.resolve(Path::new("/var/log/run.jsonl")),
        PathBuf::from("/var/log/run.jsonl")
    );
}

#[test]
fn a_missing_file_is_an_error_that_names_it() {
    let dir = dir("missing");
    let path = dir.join("missing.toml");

    let err = config::load::<Settings>(&path).unwrap_err();
    assert!(err.to_string().contains("missing.toml"), "{err}");
    let config::Error::Read {
        path: named,
        source,
    } = err
    else {
        panic!("not a read error: {err:?}");
    };
    assert_eq!(named, path);
    assert_eq!(source.kind(), io::ErrorKind::NotFound);
}

#[test]
fn a_malformed_file_is_an_error_with_its_line() {
    let dir = dir("malformed");
    let path = dir.join("malformed.toml");
    fs::write(&path, "seed = 7\nlog = \n").unwrap();

    let err = config::load::<Settings>(&path).unwrap_err();
    assert!(err.to_string().contains("malformed.toml"), "{err}");
    let cause = err.source().unwrap().to_string();
    assert!(cause.contains("line 2"), "{cause}");
}

#[test]
fn an_unknown_key_is_refused_by_name() {
    let dir = dir("unknown");
    let path = dir.join("unknown.toml");
    fs::write(&path, "seed = 7\nsede = 8\n").unwrap();

    let err = config::load::<Settings>(&path).unwrap_err();
    let cause = err.source().unwrap().to_string();
    assert!(cause.contains("sede"), "{cause}");
}
