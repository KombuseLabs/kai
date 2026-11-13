//! The store: a SQLite file that holds the tickets kai fetched, and where
//! each board's last sync stopped.
//!
//! It is a cache of GitHub, not a source of truth: deleting the file loses
//! nothing that a sync does not fetch again. So there are no migrations:
//! when the schema changes, [`Store::open`] empties a store of an older
//! schema, and the next sync fills it. The file is no contract; no other
//! program should read it.
//!
//! A file that is not kai's store is never emptied. kai marks its stores
//! with a SQLite application id and refuses a database without it that
//! holds tables.
//!
//! ```
//! use store::Store;
//!
//! let mut store = Store::in_memory()?;
//! let board = "example-org/example-board".parse()?;
//! assert_eq!(store.cursor(&board)?, None);
//! store.record_sync(&board, &[], Some("2026-01-02T03:04:05Z"))?;
//! assert_eq!(store.cursor(&board)?.as_deref(), Some("2026-01-02T03:04:05Z"));
//! assert!(store.tickets()?.is_empty());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension as _, Row, Transaction, params};
use types::{Form, IssueId, IssueState, LabelEvent, RepoName, Ticket};

/// Marks a SQLite file as kai's store (`PRAGMA application_id`): "kai1".
const APPLICATION_ID: i32 = 0x6b61_6931;

/// The schema below. Any change to it raises this number, which empties
/// every store of an older one when it is next opened.
const SCHEMA_VERSION: i32 = 1;

/// Unknown stays NULL: a form field left empty is NULL, and `has_form`
/// tells a body without the form from a form left empty.
const SCHEMA: &str = "
CREATE TABLE ticket (
    board         TEXT    NOT NULL,
    number        INTEGER NOT NULL,
    title         TEXT    NOT NULL,
    url           TEXT    NOT NULL,
    open          INTEGER NOT NULL,
    updated_at    TEXT    NOT NULL,
    has_form      INTEGER NOT NULL,
    instruction   TEXT,
    repo          TEXT,
    base_ref      TEXT,
    check_command TEXT,
    hidden_tests  TEXT,
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
CREATE TABLE sync_state (
    board TEXT NOT NULL PRIMARY KEY,
    since TEXT NOT NULL
) STRICT;
";

/// An open store.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens the store at `path`, making the file and its directory if they
    /// do not exist, and emptying a store of an older schema.
    pub fn open(path: &Path) -> Result<Store, Error> {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|source| Error::Dir {
                path: dir.to_path_buf(),
                source,
            })?;
        }
        let conn = Connection::open(path)?;
        Store::prepare(conn, path)
    }

    /// A store that lives in memory and is gone when dropped.
    pub fn in_memory() -> Result<Store, Error> {
        Store::prepare(Connection::open_in_memory()?, Path::new(":memory:"))
    }

    fn prepare(mut conn: Connection, path: &Path) -> Result<Store, Error> {
        let application_id: i32 =
            conn.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        if application_id == APPLICATION_ID && version == SCHEMA_VERSION {
            return Ok(Store { conn });
        }
        if application_id != APPLICATION_ID && !tables.is_empty() {
            return Err(Error::Foreign {
                path: path.to_path_buf(),
            });
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

    /// Records what one sync of `board` fetched: each ticket replaces the
    /// one kept for its issue, and `since`, when given, is where the next
    /// sync of `board` starts. All of it or nothing is written.
    pub fn record_sync(
        &mut self,
        board: &RepoName,
        tickets: &[Ticket],
        since: Option<&str>,
    ) -> Result<(), Error> {
        let tx = self.conn.transaction()?;
        for ticket in tickets {
            upsert(&tx, ticket)?;
        }
        if let Some(since) = since {
            tx.execute(
                "INSERT INTO sync_state (board, since) VALUES (?1, ?2)
                 ON CONFLICT (board) DO UPDATE SET since = excluded.since",
                params![board.to_string(), since],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Empties the store: every ticket and every board's cursor, so that
    /// the next sync fetches everything again. Returns how many tickets
    /// were removed.
    pub fn clear(&mut self) -> Result<usize, Error> {
        let tx = self.conn.transaction()?;
        let removed = tx.execute("DELETE FROM ticket", [])?;
        tx.execute_batch("DELETE FROM label; DELETE FROM label_event; DELETE FROM sync_state;")?;
        tx.commit()?;
        Ok(removed)
    }

    /// Where the next sync of `board` starts: the time the last one
    /// recorded, or `None` if `board` was never synced.
    pub fn cursor(&self, board: &RepoName) -> Result<Option<String>, Error> {
        Ok(self
            .conn
            .query_row(
                "SELECT since FROM sync_state WHERE board = ?1",
                [board.to_string()],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Every ticket in the store, by board and number.
    pub fn tickets(&self) -> Result<Vec<Ticket>, Error> {
        let mut tickets: Vec<Ticket> = self
            .conn
            .prepare(
                "SELECT board, number, title, url, open, updated_at, has_form,
                        instruction, repo, base_ref, check_command, hidden_tests
                 FROM ticket ORDER BY board, number",
            )?
            .query_map([], ticket_from_row)?
            .collect::<Result<_, _>>()?;
        let mut labels = self
            .conn
            .prepare("SELECT name FROM label WHERE board = ?1 AND number = ?2 ORDER BY position")?;
        let mut events = self.conn.prepare(
            "SELECT label, actor, at FROM label_event
             WHERE board = ?1 AND number = ?2 ORDER BY position",
        )?;
        for ticket in &mut tickets {
            let key = params![ticket.issue.repo.to_string(), int(ticket.issue.number)?];
            ticket.labels = labels
                .query_map(key, |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            ticket.label_events = events
                .query_map(key, |row| {
                    Ok(LabelEvent {
                        label: row.get(0)?,
                        actor: row.get(1)?,
                        at: row.get(2)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
        }
        Ok(tickets)
    }
}

fn upsert(tx: &Transaction<'_>, ticket: &Ticket) -> Result<(), Error> {
    let board = ticket.issue.repo.to_string();
    let number = int(ticket.issue.number)?;
    let form = ticket.form.as_ref();
    let field = |pick: fn(&Form) -> &Option<String>| form.and_then(|form| pick(form).clone());
    tx.execute(
        "INSERT OR REPLACE INTO ticket
           (board, number, title, url, open, updated_at, has_form,
            instruction, repo, base_ref, check_command, hidden_tests)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            board,
            number,
            ticket.title,
            ticket.url,
            ticket.state == IssueState::Open,
            ticket.updated_at,
            form.is_some(),
            field(|form| &form.instruction),
            field(|form| &form.repo),
            field(|form| &form.base_ref),
            field(|form| &form.check),
            field(|form| &form.hidden_tests),
        ],
    )?;
    tx.execute(
        "DELETE FROM label WHERE board = ?1 AND number = ?2",
        params![board, number],
    )?;
    for (position, name) in ticket.labels.iter().enumerate() {
        tx.execute(
            "INSERT INTO label (board, number, position, name) VALUES (?1, ?2, ?3, ?4)",
            params![board, number, int(position)?, name],
        )?;
    }
    tx.execute(
        "DELETE FROM label_event WHERE board = ?1 AND number = ?2",
        params![board, number],
    )?;
    for (position, event) in ticket.label_events.iter().enumerate() {
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

/// A ticket from a row of `ticket`, without its labels.
fn ticket_from_row(row: &Row<'_>) -> rusqlite::Result<Ticket> {
    let board: String = row.get(0)?;
    let repo: RepoName = board.parse().map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
    })?;
    let has_form: bool = row.get(6)?;
    let form = if has_form {
        Some(Form {
            instruction: row.get(7)?,
            repo: row.get(8)?,
            base_ref: row.get(9)?,
            check: row.get(10)?,
            hidden_tests: row.get(11)?,
        })
    } else {
        None
    };
    Ok(Ticket {
        issue: IssueId {
            repo,
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
        labels: Vec::new(),
        label_events: Vec::new(),
        updated_at: row.get(5)?,
        form,
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
