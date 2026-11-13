//! The store: what goes in comes out, a sync replaces what it fetched, a
//! full sync removes what it did not, an older schema is emptied, its
//! files are private, and another program's file is left alone.

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use store::{Error, Store};
use types::{Access, Issue, IssueId, IssueState, LabelEvent, RepoName};

/// A path of the test's own for a store file; nothing is there yet.
fn path(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("store-{name}"));
    let _ = fs::remove_dir_all(&dir);
    dir.join("deeper/store.sqlite")
}

fn board() -> RepoName {
    "example-org/example-board".parse().unwrap()
}

fn issue(number: u64) -> Issue {
    Issue {
        id: IssueId {
            repo: board(),
            number,
        },
        title: format!("Invented issue {number}"),
        url: format!("https://github.com/example-org/example-board/issues/{number}"),
        state: IssueState::Open,
        author: Some("example-author".to_owned()),
        created_at: "2026-01-01T00:00:00Z".to_owned(),
        updated_at: "2026-01-02T03:04:06Z".to_owned(),
        body: "### Instruction\n\nInvented instruction.\n".to_owned(),
        labels: vec!["kai".to_owned(), "bug".to_owned()],
        labels_complete: true,
        label_events: vec![
            LabelEvent {
                label: "kai".to_owned(),
                actor: Some("example-maintainer".to_owned()),
                at: "2026-01-02T03:04:05Z".to_owned(),
            },
            LabelEvent {
                label: "bug".to_owned(),
                actor: None,
                at: "2026-01-02T03:04:06Z".to_owned(),
            },
        ],
        label_events_complete: true,
    }
}

#[test]
fn issues_come_out_as_they_went_in_by_number() {
    let path = path("roundtrip");
    let mut plain = issue(1);
    plain.body = String::new();
    plain.author = None;
    plain.state = IssueState::Closed;
    plain.labels.clear();
    let mut cut = issue(3);
    cut.labels_complete = false;
    cut.label_events_complete = false;
    let issues = vec![issue(2), cut, plain];

    let mut store = Store::open(&path).unwrap();
    store
        .record_sync(&board(), &issues, Some("2026-01-02T03:04:06Z"), false)
        .unwrap();
    drop(store);

    let store = Store::open(&path).unwrap();
    let mut expected = issues;
    expected.sort_by_key(|issue| issue.id.number);
    assert_eq!(store.issues().unwrap(), expected);
}

#[test]
fn an_issue_synced_again_replaces_the_one_kept_labels_and_all() {
    let mut store = Store::in_memory().unwrap();
    store
        .record_sync(&board(), &[issue(1)], None, false)
        .unwrap();
    let mut changed = issue(1);
    changed.title = "Invented, renamed".to_owned();
    changed.labels = vec!["kai".to_owned()];
    changed.label_events.truncate(1);
    changed.body = "Rewritten.".to_owned();
    store
        .record_sync(&board(), &[changed.clone()], None, false)
        .unwrap();
    assert_eq!(store.issues().unwrap(), [changed]);
}

#[test]
fn the_cursor_is_kept_per_board() {
    let mut store = Store::in_memory().unwrap();
    let other: RepoName = "example-org/other-board".parse().unwrap();
    store
        .record_sync(&board(), &[], Some("2026-01-01T00:00:00Z"), false)
        .unwrap();
    store
        .record_sync(&board(), &[], Some("2026-02-01T00:00:00Z"), false)
        .unwrap();
    // A sync that fetched nothing and had no cursor keeps the one there.
    store.record_sync(&board(), &[], None, false).unwrap();
    store.record_sync(&other, &[], None, false).unwrap();
    assert_eq!(
        store.cursor(&board()).unwrap().as_deref(),
        Some("2026-02-01T00:00:00Z")
    );
    assert_eq!(store.cursor(&other).unwrap(), None);
}

#[test]
fn only_a_full_sync_removes_what_github_no_longer_returns() {
    let mut store = Store::in_memory().unwrap();
    let other: RepoName = "example-org/other-board".parse().unwrap();
    let mut elsewhere = issue(1);
    elsewhere.id.repo = other.clone();
    store
        .record_sync(&other, &[elsewhere.clone()], None, true)
        .unwrap();
    store
        .record_sync(&board(), &[issue(1), issue(2), issue(3)], None, false)
        .unwrap();

    let removed = store
        .record_sync(&board(), &[issue(1)], None, false)
        .unwrap();
    assert_eq!(removed, 0);
    assert_eq!(store.issues().unwrap().len(), 4);

    let removed = store
        .record_sync(&board(), &[issue(1), issue(3)], None, true)
        .unwrap();
    assert_eq!(removed, 1);
    let left: Vec<String> = store
        .issues()
        .unwrap()
        .iter()
        .map(|issue| issue.id.to_string())
        .collect();
    assert_eq!(
        left,
        [
            "example-org/example-board#1",
            "example-org/example-board#3",
            "example-org/other-board#1"
        ]
    );
}

#[test]
fn each_board_says_when_it_was_synced_and_fully_synced() {
    let mut store = Store::in_memory().unwrap();
    store
        .record_sync(&board(), &[issue(1)], Some("2026-01-02T03:04:06Z"), false)
        .unwrap();
    let [synced] = &store.boards().unwrap()[..] else {
        unreachable!()
    };
    assert_eq!(synced.board, board());
    assert_eq!(synced.since.as_deref(), Some("2026-01-02T03:04:06Z"));
    assert_eq!(synced.last_full_sync, None);
    assert_eq!(synced.issues, 1);
    // RFC 3339 in UTC, whole seconds, as GitHub writes its times.
    assert_eq!(synced.last_sync.len(), "2026-01-02T03:04:06Z".len());
    assert!(synced.last_sync.ends_with('Z'));

    store.record_sync(&board(), &[], None, true).unwrap();
    store.record_sync(&board(), &[], None, false).unwrap();
    let [synced] = &store.boards().unwrap()[..] else {
        unreachable!()
    };
    assert!(synced.last_full_sync.is_some());
    assert_eq!(synced.issues, 0);
}

#[test]
fn clear_removes_every_issue_and_cursor() {
    let mut store = Store::in_memory().unwrap();
    store
        .record_sync(
            &board(),
            &[issue(1), issue(2)],
            Some("2026-01-01T00:00:00Z"),
            false,
        )
        .unwrap();
    assert_eq!(store.clear().unwrap(), 2);
    assert!(store.issues().unwrap().is_empty());
    assert_eq!(store.cursor(&board()).unwrap(), None);
    assert!(store.boards().unwrap().is_empty());
    assert_eq!(store.clear().unwrap(), 0);
}

#[test]
fn a_store_kai_makes_is_its_owners_alone() {
    let path = path("modes");
    Store::open(&path).unwrap();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
}

#[test]
fn a_file_that_is_there_keeps_its_mode() {
    let path = path("kept-mode");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    Store::open(&path).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn a_store_of_an_older_schema_is_emptied() {
    let path = path("older");
    let mut store = Store::open(&path).unwrap();
    store
        .record_sync(&board(), &[issue(1)], Some("2026-01-01T00:00:00Z"), false)
        .unwrap();
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", 0).unwrap();
    conn.execute_batch("CREATE TABLE gone (x INTEGER)").unwrap();
    drop(conn);

    let store = Store::open(&path).unwrap();
    assert!(store.issues().unwrap().is_empty());
    assert_eq!(store.cursor(&board()).unwrap(), None);
}

#[test]
fn another_programs_database_is_refused_and_left_alone() {
    let path = path("foreign");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE precious (x INTEGER); INSERT INTO precious VALUES (7);")
        .unwrap();
    drop(conn);

    assert!(matches!(Store::open(&path), Err(Error::Foreign { .. })));
    let conn = rusqlite::Connection::open(&path).unwrap();
    let x: i64 = conn
        .query_row("SELECT x FROM precious", [], |row| row.get(0))
        .unwrap();
    assert_eq!(x, 7);
    let journal: String = conn
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "delete");
}

#[test]
fn a_file_that_is_no_database_is_refused_and_left_alone() {
    let path = path("text");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text = "not a database, but long enough to have a header of its own.\n".repeat(3);
    fs::write(&path, &text).unwrap();

    assert!(matches!(Store::open(&path), Err(Error::Sqlite(_))));
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn the_access_of_each_label_actor_is_kept_and_replaced_as_a_whole() {
    let mut store = Store::in_memory().unwrap();
    let mut other = issue(2);
    other.label_events[1].actor = Some("example-triager".to_owned());
    store
        .record_sync(&board(), &[issue(1), other], None, false)
        .unwrap();
    // The deleted account of issue 1's second event has no login.
    assert_eq!(
        store.actors(&board()).unwrap(),
        ["example-maintainer", "example-triager"]
    );

    let maintainer = Access::Known {
        permission: "admin".to_owned(),
        role: Some("admin".to_owned()),
    };
    let triager = Access::Unknown("GitHub answered 404: invented".to_owned());
    store
        .record_access(
            &board(),
            &[
                ("example-triager".to_owned(), triager.clone()),
                ("example-maintainer".to_owned(), maintainer.clone()),
            ],
        )
        .unwrap();
    let kept = store.access(&board()).unwrap();
    let pairs: Vec<(&str, &Access)> = kept
        .iter()
        .map(|actor| (actor.login.as_str(), &actor.access))
        .collect();
    assert_eq!(
        pairs,
        [
            ("example-maintainer", &maintainer),
            ("example-triager", &triager)
        ]
    );
    assert!(kept[0].observed_at.ends_with('Z'));

    store
        .record_access(&board(), &[("example-maintainer".to_owned(), maintainer)])
        .unwrap();
    assert_eq!(store.access(&board()).unwrap().len(), 1);
    store.clear().unwrap();
    assert!(store.access(&board()).unwrap().is_empty());
}
