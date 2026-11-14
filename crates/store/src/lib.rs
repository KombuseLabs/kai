//! The store: a SQLite file that mirrors the issues kai fetched from
//! GitHub, as GitHub said them, and when and where each board's syncs
//! stopped.
//!
//! It holds GitHub's data and nothing kai made of it: an issue's body is
//! kept whole, and reading it as the kai ticket form is
//! `engine::ticket`'s. So everything in it can be fetched again, and when
//! the schema changes, [`Store::open`] empties a store of an older one and
//! the next sync fills it: there are no migrations. What kai records of
//! its own never goes in here. The file is no contract; no other program
//! should read it.
//!
//! It also keeps what GitHub said of the access of each account that put
//! a label on, and when it was asked: the trust gate's data. Each time it
//! is asked, the answers replace those of before; an access GitHub did not
//! tell is kept with GitHub's words, never as no access.
//!
//! And it keeps the files of each board at the head of its default branch,
//! as GitHub listed them last: where the hidden tests of its tickets lie.
//!
//! A file that is not kai's store is never emptied, nor changed at all.
//! kai marks its stores with a SQLite application id and refuses a
//! database without it that holds tables.
//!
//! A file the store makes is readable by its owner only (`0600`), and so
//! is every directory it makes for it (`0700`): it holds the text of
//! issues, which may be a private board's. A file or directory that is
//! already there keeps its mode. The store waits up to five seconds for
//! another process that writes it, and writes ahead (WAL), so that one
//! command can read while another syncs.
//!
//! ```
//! use store::Store;
//!
//! let mut store = Store::in_memory()?;
//! let board = "example-org/example-board".parse()?;
//! assert_eq!(store.cursor(&board)?, None);
//! store.record_sync(&board, &[], Some("2026-01-02T03:04:05Z"), false)?;
//! assert_eq!(store.cursor(&board)?.as_deref(), Some("2026-01-02T03:04:05Z"));
//! assert!(store.issues()?.is_empty());
//! let synced = store.boards()?;
//! assert_eq!(synced[0].last_full_sync, None);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::BTreeSet;
use std::fs::{DirBuilder, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension as _, Row, Transaction, params};
use types::{Access, ActorAccess, BoardFiles, Issue, IssueId, IssueState, LabelEvent, RepoName};

/// Marks a SQLite file as kai's store (`PRAGMA application_id`): "kai1".
const APPLICATION_ID: i32 = 0x6b61_6931;

/// The schema below. Any change to it raises this number, which empties
/// every store of an older one when it is next opened.
const SCHEMA_VERSION: i32 = 4;

/// How long a command waits for another process that holds the file.
const BUSY: Duration = Duration::from_secs(5);

/// The time now as SQLite writes it, in the shape GitHub writes its
/// times (RFC 3339, UTC, whole seconds).
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";

/// Unknown stays NULL: an issue without an author is a deleted account,
/// and a board's `since` is NULL until a sync fetched something.
const SCHEMA: &str = "
CREATE TABLE issue (
    board                 TEXT    NOT NULL,
    number                INTEGER NOT NULL,
    title                 TEXT    NOT NULL,
    url                   TEXT    NOT NULL,
    open                  INTEGER NOT NULL,
    author                TEXT,
    created_at            TEXT    NOT NULL,
    updated_at            TEXT    NOT NULL,
    body                  TEXT    NOT NULL,
    labels_complete       INTEGER NOT NULL,
    label_events_complete INTEGER NOT NULL,
    PRIMARY KEY (board, number)
) STRICT;
CREATE TABLE label (
    board    TEXT    NOT NULL,
    number   INTEGER NOT NULL,
    position INTEGER NOT NULL,
    name     TEXT    NOT NULL,
    PRIMARY KEY (board, number, position)
) STRICT;
CREATE TABLE label_event (
    board    TEXT    NOT NULL,
    number   INTEGER NOT NULL,
    position INTEGER NOT NULL,
    label    TEXT    NOT NULL,
    actor    TEXT,
    at       TEXT    NOT NULL,
    PRIMARY KEY (board, number, position)
) STRICT;
CREATE TABLE actor_access (
    board       TEXT NOT NULL,
    login       TEXT NOT NULL,
    permission  TEXT,
    role        TEXT,
    unknown     TEXT,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (board, login),
    CHECK ((permission IS NULL) <> (unknown IS NULL))
) STRICT;
CREATE TABLE board_tree (
    board       TEXT    NOT NULL PRIMARY KEY,
    branch      TEXT    NOT NULL,
    head        TEXT    NOT NULL,
    complete    INTEGER NOT NULL,
    observed_at TEXT    NOT NULL
) STRICT;
CREATE TABLE board_file (
    board    TEXT    NOT NULL,
    position INTEGER NOT NULL,
    path     TEXT    NOT NULL,
    PRIMARY KEY (board, position)
) STRICT;
CREATE TABLE sync_state (
    board          TEXT NOT NULL PRIMARY KEY,
    since          TEXT,
    last_sync      TEXT NOT NULL,
    last_full_sync TEXT
) STRICT;
";

/// A board's files as GitHub listed them last, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedFiles {
    /// The files.
    pub files: BoardFiles,
    /// When GitHub listed them (RFC 3339, UTC).
    pub observed_at: String,
}

/// An open store.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

/// What the store knows of one board's syncs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardSync {
    /// The board.
    pub board: RepoName,
    /// Where the next sync starts: the latest change a sync fetched, or
    /// `None` when none fetched anything.
    pub since: Option<String>,
    /// When the last sync of the board was recorded (RFC 3339, UTC).
    pub last_sync: String,
    /// When the last full sync was recorded, or `None` when there was
    /// none. Only a full sync removes issues GitHub no longer returns.
    pub last_full_sync: Option<String>,
    /// How many of the board's issues the store holds.
    pub issues: usize,
}

impl Store {
    /// Opens the store at `path`, making the file and its directory if they
    /// do not exist (readable by their owner only), and emptying a store of
    /// an older schema.
    pub fn open(path: &Path) -> Result<Store, Error> {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|source| Error::Dir {
                    path: dir.to_path_buf(),
                    source,
                })?;
        }
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(Error::File {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
        let conn = Connection::open(path)?;
        Store::prepare(conn, path)
    }

    /// A store that lives in memory and is gone when dropped.
    pub fn in_memory() -> Result<Store, Error> {
        Store::prepare(Connection::open_in_memory()?, Path::new(":memory:"))
    }

    fn prepare(mut conn: Connection, path: &Path) -> Result<Store, Error> {
        conn.busy_timeout(BUSY)?;
        let application_id: i32 =
            conn.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        if application_id != APPLICATION_ID && !tables.is_empty() {
            return Err(Error::Foreign {
                path: path.to_path_buf(),
            });
        }
        // Only now, since the file is kai's: the journal mode is written
        // into the file.
        let _mode: String =
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
        if application_id == APPLICATION_ID && version == SCHEMA_VERSION {
            return Ok(Store { conn });
        }
        if !tables.is_empty() {
            tracing::info!(
                path = %path.display(),
                found = version,
                schema = SCHEMA_VERSION,
                "the store has an older schema; it starts empty, and a sync fills it again"
            );
        }
        let tx = conn.transaction()?;
        for table in &tables {
            tx.execute_batch(&format!("DROP TABLE \"{}\";", table.replace('"', "\"\"")))?;
        }
        tx.execute_batch(SCHEMA)?;
        tx.pragma_update(None, "application_id", APPLICATION_ID)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        tx.commit()?;
        Ok(Store { conn })
    }

    /// Records what one sync of `board` fetched: each issue replaces the
    /// one kept for it, and `since`, when given, is where the next sync of
    /// `board` starts. With `full`, the sync fetched every issue of the
    /// board, and those the store holds that it did not fetch are removed
    /// (deleted or moved on GitHub); returns how many. All of it or
    /// nothing is written.
    pub fn record_sync(
        &mut self,
        board: &RepoName,
        issues: &[Issue],
        since: Option<&str>,
        full: bool,
    ) -> Result<usize, Error> {
        let board_name = board.to_string();
        let tx = self.conn.transaction()?;
        for issue in issues {
            upsert(&tx, &board_name, issue)?;
        }
        let mut removed = 0;
        if full {
            let fetched: BTreeSet<u64> = issues.iter().map(|issue| issue.id.number).collect();
            let kept: Vec<i64> = tx
                .prepare("SELECT number FROM issue WHERE board = ?1")?
                .query_map([&board_name], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            for number in kept {
                let gone = u64::try_from(number).is_ok_and(|n| !fetched.contains(&n));
                if gone {
                    for table in ["issue", "label", "label_event"] {
                        tx.execute(
                            &format!("DELETE FROM {table} WHERE board = ?1 AND number = ?2"),
                            params![board_name, number],
                        )?;
                    }
                    removed += 1;
                }
            }
        }
        tx.execute(
            &format!(
                "INSERT INTO sync_state (board, since, last_sync, last_full_sync)
                 VALUES (?1, ?2, {NOW}, CASE WHEN ?3 THEN {NOW} END)
                 ON CONFLICT (board) DO UPDATE SET
                   since = coalesce(excluded.since, since),
                   last_sync = excluded.last_sync,
                   last_full_sync = coalesce(excluded.last_full_sync, last_full_sync)"
            ),
            params![board_name, since, full],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    /// Empties the store: every issue and what it knew of every board's
    /// syncs, so that the next sync fetches everything again. Returns how
    /// many issues were removed.
    pub fn clear(&mut self) -> Result<usize, Error> {
        let tx = self.conn.transaction()?;
        let removed = tx.execute("DELETE FROM issue", [])?;
        tx.execute_batch(
            "DELETE FROM label; DELETE FROM label_event; DELETE FROM actor_access;
             DELETE FROM board_tree; DELETE FROM board_file; DELETE FROM sync_state;",
        )?;
        tx.commit()?;
        Ok(removed)
    }

    /// Where the next sync of `board` starts: the time the last one
    /// recorded, or `None` if no sync of `board` fetched anything.
    pub fn cursor(&self, board: &RepoName) -> Result<Option<String>, Error> {
        Ok(self
            .conn
            .query_row(
                "SELECT since FROM sync_state WHERE board = ?1",
                [board.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Every account that put a label on an issue of `board` the store
    /// holds, by login; a deleted account, which has none, is left out.
    pub fn actors(&self, board: &RepoName) -> Result<Vec<String>, Error> {
        Ok(self
            .conn
            .prepare(
                "SELECT DISTINCT actor FROM label_event
                 WHERE board = ?1 AND actor IS NOT NULL ORDER BY actor",
            )?
            .query_map([board.to_string()], |row| row.get(0))?
            .collect::<Result<_, _>>()?)
    }

    /// Records what GitHub said now of each account's access to `board`,
    /// in place of everything it said before for that board.
    pub fn record_access(
        &mut self,
        board: &RepoName,
        answers: &[(String, Access)],
    ) -> Result<(), Error> {
        let board = board.to_string();
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM actor_access WHERE board = ?1", [&board])?;
        for (login, access) in answers {
            let (permission, role, unknown) = match access {
                Access::Known { permission, role } => (Some(permission), role.as_ref(), None),
                Access::Unknown(why) => (None, None, Some(why)),
            };
            tx.execute(
                &format!(
                    "INSERT INTO actor_access (board, login, permission, role, unknown, observed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, {NOW})"
                ),
                params![board, login, permission, role, unknown],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// What GitHub said last of the access of each account to `board`, by
    /// login.
    pub fn access(&self, board: &RepoName) -> Result<Vec<ActorAccess>, Error> {
        Ok(self
            .conn
            .prepare(
                "SELECT login, permission, role, unknown, observed_at FROM actor_access
                 WHERE board = ?1 ORDER BY login",
            )?
            .query_map([board.to_string()], |row| {
                let permission: Option<String> = row.get(1)?;
                let access = match permission {
                    Some(permission) => Access::Known {
                        permission,
                        role: row.get(2)?,
                    },
                    None => Access::Unknown(row.get(3)?),
                };
                Ok(ActorAccess {
                    login: row.get(0)?,
                    access,
                    observed_at: row.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?)
    }

    /// Records the files of `board` as GitHub listed them now, in place of
    /// those of before.
    pub fn record_board_files(
        &mut self,
        board: &RepoName,
        files: &BoardFiles,
    ) -> Result<(), Error> {
        let board = board.to_string();
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM board_file WHERE board = ?1", [&board])?;
        tx.execute(
            &format!(
                "INSERT OR REPLACE INTO board_tree (board, branch, head, complete, observed_at)
                 VALUES (?1, ?2, ?3, ?4, {NOW})"
            ),
            params![board, files.branch, files.commit, files.complete],
        )?;
        for (position, path) in files.paths.iter().enumerate() {
            tx.execute(
                "INSERT INTO board_file (board, position, path) VALUES (?1, ?2, ?3)",
                params![board, int(position)?, path],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The files of `board` as GitHub listed them last, and when, or
    /// `None` when they were never listed.
    pub fn board_files(&self, board: &RepoName) -> Result<Option<ListedFiles>, Error> {
        let board = board.to_string();
        let head = self
            .conn
            .query_row(
                "SELECT branch, head, complete, observed_at FROM board_tree WHERE board = ?1",
                [&board],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((branch, commit, complete, observed_at)) = head else {
            return Ok(None);
        };
        let paths = self
            .conn
            .prepare("SELECT path FROM board_file WHERE board = ?1 ORDER BY position")?
            .query_map([&board], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(Some(ListedFiles {
            files: BoardFiles {
                branch,
                commit,
                paths,
                complete,
            },
            observed_at,
        }))
    }

    /// Every board the store has a sync of, by name.
    pub fn boards(&self) -> Result<Vec<BoardSync>, Error> {
        let boards = self
            .conn
            .prepare(
                "SELECT s.board, s.since, s.last_sync, s.last_full_sync,
                        (SELECT count(*) FROM issue i WHERE i.board = s.board)
                 FROM sync_state s ORDER BY s.board",
            )?
            .query_map([], |row| {
                Ok(BoardSync {
                    board: repo(row, 0)?,
                    since: row.get(1)?,
                    last_sync: row.get(2)?,
                    last_full_sync: row.get(3)?,
                    issues: usize::try_from(row.get::<_, i64>(4)?).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Integer,
                            Box::new(err),
                        )
                    })?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(boards)
    }

    /// Every issue in the store, by board and number.
    pub fn issues(&self) -> Result<Vec<Issue>, Error> {
        let mut issues: Vec<Issue> = self
            .conn
            .prepare(
                "SELECT board, number, title, url, open, author, created_at, updated_at,
                        body, labels_complete, label_events_complete
                 FROM issue ORDER BY board, number",
            )?
            .query_map([], issue_from_row)?
            .collect::<Result<_, _>>()?;
        let mut labels = self
            .conn
            .prepare("SELECT name FROM label WHERE board = ?1 AND number = ?2 ORDER BY position")?;
        let mut events = self.conn.prepare(
            "SELECT label, actor, at FROM label_event
             WHERE board = ?1 AND number = ?2 ORDER BY position",
        )?;
        for issue in &mut issues {
            let key = params![issue.id.repo.to_string(), int(issue.id.number)?];
            issue.labels = labels
                .query_map(key, |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            issue.label_events = events
                .query_map(key, |row| {
                    Ok(LabelEvent {
                        label: row.get(0)?,
                        actor: row.get(1)?,
                        at: row.get(2)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
        }
        Ok(issues)
    }
}

fn upsert(tx: &Transaction<'_>, board: &str, issue: &Issue) -> Result<(), Error> {
    let number = int(issue.id.number)?;
    tx.execute(
        "INSERT OR REPLACE INTO issue
           (board, number, title, url, open, author, created_at, updated_at,
            body, labels_complete, label_events_complete)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            board,
            number,
            issue.title,
            issue.url,
            issue.state == IssueState::Open,
            issue.author,
            issue.created_at,
            issue.updated_at,
            issue.body,
            issue.labels_complete,
            issue.label_events_complete,
        ],
    )?;
    tx.execute(
        "DELETE FROM label WHERE board = ?1 AND number = ?2",
        params![board, number],
    )?;
    for (position, name) in issue.labels.iter().enumerate() {
        tx.execute(
            "INSERT INTO label (board, number, position, name) VALUES (?1, ?2, ?3, ?4)",
            params![board, number, int(position)?, name],
        )?;
    }
    tx.execute(
        "DELETE FROM label_event WHERE board = ?1 AND number = ?2",
        params![board, number],
    )?;
    for (position, event) in issue.label_events.iter().enumerate() {
        tx.execute(
            "INSERT INTO label_event (board, number, position, label, actor, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                board,
                number,
                int(position)?,
                event.label,
                event.actor,
                event.at
            ],
        )?;
    }
    Ok(())
}

/// `n` as SQLite's integer, which is signed.
fn int<N: TryInto<i64, Error = std::num::TryFromIntError>>(n: N) -> rusqlite::Result<i64> {
    n.try_into()
        .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))
}

/// The board in column `index`, read back as a repository name.
fn repo(row: &Row<'_>, index: usize) -> rusqlite::Result<RepoName> {
    let board: String = row.get(index)?;
    board.parse().map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(err))
    })
}

/// An issue from a row of `issue`, without its labels.
fn issue_from_row(row: &Row<'_>) -> rusqlite::Result<Issue> {
    Ok(Issue {
        id: IssueId {
            repo: repo(row, 0)?,
            number: u64::try_from(row.get::<_, i64>(1)?).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Integer,
                    Box::new(err),
                )
            })?,
        },
        title: row.get(2)?,
        url: row.get(3)?,
        state: if row.get(4)? {
            IssueState::Open
        } else {
            IssueState::Closed
        },
        author: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        body: row.get(8)?,
        labels: Vec::new(),
        labels_complete: row.get(9)?,
        label_events: Vec::new(),
        label_events_complete: row.get(10)?,
    })
}

/// Why the store could not be opened, read or written.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The directory the store goes in cannot be made.
    #[error("cannot make the directory {}", .path.display())]
    Dir {
        /// The directory.
        path: PathBuf,
        /// Why it cannot be made.
        source: io::Error,
    },
    /// The store's file cannot be made.
    #[error("cannot make the file {}", .path.display())]
    File {
        /// The file.
        path: PathBuf,
        /// Why it cannot be made.
        source: io::Error,
    },
    /// The file is another program's SQLite database; kai leaves it alone.
    #[error("{} is not kai's store but another program's database; kai leaves it alone", .path.display())]
    Foreign {
        /// The file.
        path: PathBuf,
    },
    /// SQLite refused: the file is not a database, is locked, or holds
    /// what kai does not read.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}
