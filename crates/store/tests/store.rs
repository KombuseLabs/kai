//! The store: what goes in comes out, a sync replaces what it fetched, an
//! older schema is emptied, and another program's file is left alone.

use std::fs;
use std::path::{Path, PathBuf};

use store::{Error, Store};
use types::{Form, IssueId, IssueState, LabelEvent, RepoName, Ticket};

/// A path of the test's own for a store file; nothing is there yet.
fn path(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("store-{name}"));
    let _ = fs::remove_dir_all(&dir);
    dir.join("deeper/store.sqlite")
}

fn board() -> RepoName {
    "example-org/example-board".parse().unwrap()
}

fn ticket(number: u64) -> Ticket {
    Ticket {
        issue: IssueId {
            repo: board(),
            number,
        },
        title: format!("Invented ticket {number}"),
        url: format!("https://github.com/example-org/example-board/issues/{number}"),
        state: IssueState::Open,
        labels: vec!["kai".to_owned(), "bug".to_owned()],
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
        updated_at: "2026-01-02T03:04:06Z".to_owned(),
        form: Some(Form {
            instruction: Some("Invented instruction.".to_owned()),
            repo: Some("example-org/example-code".to_owned()),
            base_ref: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            check: None,
            hidden_tests: Some("tests/hidden/invented.rs".to_owned()),
        }),
    }
}

#[test]
fn tickets_come_out_as_they_went_in_by_number() {
    let path = path("roundtrip");
    let mut no_form = ticket(1);
    no_form.form = None;
    no_form.state = IssueState::Closed;
    no_form.labels.clear();
    let mut empty_form = ticket(3);
    empty_form.form = Some(Form::default());
    let tickets = vec![ticket(2), empty_form, no_form];

    let mut store = Store::open(&path).unwrap();
    store
        .record_sync(&board(), &tickets, Some("2026-01-02T03:04:06Z"))
        .unwrap();
    drop(store);

    let store = Store::open(&path).unwrap();
    let mut expected = tickets;
    expected.sort_by_key(|ticket| ticket.issue.number);
    assert_eq!(store.tickets().unwrap(), expected);
}

#[test]
fn a_ticket_synced_again_replaces_the_one_kept_labels_and_all() {
    let mut store = Store::in_memory().unwrap();
    store.record_sync(&board(), &[ticket(1)], None).unwrap();
    let mut changed = ticket(1);
    changed.title = "Invented, renamed".to_owned();
    changed.labels = vec!["kai".to_owned()];
    changed.label_events.truncate(1);
    changed.form.as_mut().unwrap().check = Some("cargo test".to_owned());
    store
        .record_sync(&board(), &[changed.clone()], None)
        .unwrap();
    assert_eq!(store.tickets().unwrap(), [changed]);
}

#[test]
fn the_cursor_is_kept_per_board() {
    let mut store = Store::in_memory().unwrap();
    let other: RepoName = "example-org/other-board".parse().unwrap();
    store
        .record_sync(&board(), &[], Some("2026-01-01T00:00:00Z"))
        .unwrap();
    store
        .record_sync(&board(), &[], Some("2026-02-01T00:00:00Z"))
        .unwrap();
    store.record_sync(&other, &[], None).unwrap();
    assert_eq!(
        store.cursor(&board()).unwrap().as_deref(),
        Some("2026-02-01T00:00:00Z")
    );
    assert_eq!(store.cursor(&other).unwrap(), None);
}

#[test]
fn clear_removes_every_ticket_and_cursor() {
    let mut store = Store::in_memory().unwrap();
    store
        .record_sync(
            &board(),
            &[ticket(1), ticket(2)],
            Some("2026-01-01T00:00:00Z"),
        )
        .unwrap();
    assert_eq!(store.clear().unwrap(), 2);
    assert!(store.tickets().unwrap().is_empty());
    assert_eq!(store.cursor(&board()).unwrap(), None);
    assert_eq!(store.clear().unwrap(), 0);
}

#[test]
fn a_store_of_an_older_schema_is_emptied() {
    let path = path("older");
    let mut store = Store::open(&path).unwrap();
    store
        .record_sync(&board(), &[ticket(1)], Some("2026-01-01T00:00:00Z"))
        .unwrap();
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", 0).unwrap();
    conn.execute_batch("CREATE TABLE gone (x INTEGER)").unwrap();
    drop(conn);

    let store = Store::open(&path).unwrap();
    assert!(store.tickets().unwrap().is_empty());
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
