//! The ticket mirror's HTTP client. Hatch exposes ticketz's commands; kai
//! reads versioned JSON records, never the mirror's database or credentials.
//! A snapshot contains issues and board observations from one read transaction.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use types::{Access, ActorAccess, BoardFiles, Issue, IssueId, IssueState, LabelEvent, RepoName};

/// The default local hatch service.
pub const DEFAULT_URL: &str = "http://127.0.0.1:4287";
/// Maximum HTTP response, including hatch's JSON-escaped stdout.
const MAX_RESPONSE: usize = 32 * 1024 * 1024;

/// A client of the configured ticket mirror. It holds no credential.
#[derive(Debug)]
pub struct Store {
    http: reqwest::Client,
    url: String,
}

/// Issues and board observations from one consistent mirror snapshot.
#[derive(Debug)]
pub struct Snapshot {
    /// Issues as GitHub returned them, still without interpreting their forms.
    pub issues: Vec<Issue>,
    /// Boards, their sync timestamps, actor access and file listing.
    pub boards: Vec<Board>,
}

/// A board's cached observations.
#[derive(Debug)]
pub struct Board {
    /// Repository holding the issues.
    pub repo: RepoName,
    /// Number of issues in this snapshot.
    pub issues: usize,
    /// Most recent issue-change cursor, absent for an empty board.
    pub cursor: Option<String>,
    /// When this board was last synced.
    pub last_sync: String,
    /// When a full sync last replaced this board's issues.
    pub last_full_sync: Option<String>,
    /// Access observed for the board's label actors.
    pub access: Vec<ActorAccess>,
    /// Last successful file listing, or unknown.
    pub files: Option<BoardFiles>,
}

/// The completed sync, including partial ancillary evidence.
#[derive(Debug, Deserialize)]
pub struct Sync {
    version: String,
    /// Repository just synced.
    pub repo: String,
    /// Whether this was a full replacement.
    pub full: bool,
    /// Issues fetched from GitHub.
    pub fetched: usize,
    /// Issues now held in the mirror.
    pub kept: usize,
    /// Removed issues, known only for a full sync.
    pub removed: Option<usize>,
    /// Why optional board-file discovery could not refresh its observation.
    pub board_files_unknown: Option<String>,
}

impl Store {
    /// Connects to the configured hatch base URL. No request is sent yet.
    pub fn new(url: &str) -> Result<Self, Error> {
        let parsed =
            reqwest::Url::parse(url).map_err(|_| Error::Protocol("invalid store URL".into()))?;
        if !helpers::http::allowed_url(url)
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(Error::Protocol(
                "store URL must be HTTPS or loopback HTTP, without credentials, query or fragment"
                    .into(),
            ));
        }
        let http = helpers::http::builder(concat!("kai/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(310))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(transport)?;
        Ok(Self {
            http,
            url: url.trim_end_matches('/').to_owned(),
        })
    }

    /// Reads issues and qualification evidence together, rejecting failed,
    /// truncated, incompatible or internally inconsistent responses.
    pub async fn snapshot(&self) -> Result<Snapshot, Error> {
        let raw: WireSnapshot = self.call("snapshot", json!({})).await?;
        version(&raw.version, "ticketz-snapshot/1")?;
        let mut boards = Vec::new();
        let mut names = BTreeSet::new();
        for raw in raw.boards {
            version(&raw.version, "ticketz-status/1")?;
            let repo: RepoName = raw
                .repo
                .parse()
                .map_err(|e| Error::Protocol(format!("invalid board: {e}")))?;
            if !names.insert(repo.clone()) {
                return Err(Error::Protocol("duplicate board".into()));
            }
            let access = raw
                .actors
                .into_iter()
                .map(|(login, a)| a.into_access(login))
                .collect::<Result<_, _>>()?;
            let files = raw.board_files.map(|f| BoardFiles {
                branch: f.branch,
                commit: f.commit,
                paths: f.paths,
                complete: f.complete,
            });
            boards.push(Board {
                repo,
                issues: raw.issues,
                cursor: raw.cursor,
                last_sync: raw.last_sync,
                last_full_sync: raw.last_full_sync,
                access,
                files,
            });
        }
        let mut issues = Vec::new();
        let mut ids = BTreeSet::new();
        for raw in raw.issues {
            version(&raw.version, "ticketz-issue/1")?;
            let repo: RepoName = raw
                .repo
                .parse()
                .map_err(|e| Error::Protocol(format!("invalid issue repository: {e}")))?;
            if !names.contains(&repo) || raw.number == 0 {
                return Err(Error::Protocol(
                    "issue has no board or has a zero number".into(),
                ));
            }
            let id = IssueId {
                repo,
                number: raw.number,
            };
            if !ids.insert(id.clone()) {
                return Err(Error::Protocol("duplicate issue".into()));
            }
            let state = match raw.state.as_str() {
                "open" => IssueState::Open,
                "closed" => IssueState::Closed,
                _ => return Err(Error::Protocol("unknown issue state".into())),
            };
            issues.push(Issue {
                id,
                title: raw.title,
                url: raw.url,
                state,
                author: raw.author,
                created_at: raw.created_at,
                updated_at: raw.updated_at,
                body: raw.body,
                labels: raw.labels,
                labels_complete: raw.labels_complete,
                label_events: raw
                    .label_events
                    .into_iter()
                    .map(|e| LabelEvent {
                        label: e.label,
                        actor: e.actor,
                        at: e.at,
                    })
                    .collect(),
                label_events_complete: raw.label_events_complete,
            });
        }
        for board in &boards {
            if board.issues != issues.iter().filter(|i| i.id.repo == board.repo).count() {
                return Err(Error::Protocol(
                    "board issue count disagrees with snapshot".into(),
                ));
            }
        }
        Ok(Snapshot { issues, boards })
    }

    /// Requests a GitHub read through the service's own proxy and credentials.
    pub async fn sync(
        &self,
        repo: &RepoName,
        full: bool,
        actor_permissions: bool,
    ) -> Result<Sync, Error> {
        let result: Sync = self
            .call(
                "sync",
                json!({
                    "repo": repo.to_string(),
                    "full": full,
                    "actor-permissions": actor_permissions,
                    "json": true,
                }),
            )
            .await?;
        version(&result.version, "ticketz-sync/1")?;
        if result.repo != repo.to_string()
            || (full && !result.full)
            || (result.full && result.removed.is_none())
            || (!result.full && result.removed.is_some())
        {
            return Err(Error::Protocol("inconsistent sync summary".into()));
        }
        Ok(result)
    }

    /// Clears the remote cache, through ticketz's foreign-database checks.
    pub async fn clear(&self) -> Result<usize, Error> {
        #[derive(Deserialize)]
        struct Reset {
            version: String,
            removed: usize,
        }
        let result: Reset = self.call("reset", json!({"json":true})).await?;
        version(&result.version, "ticketz-reset/1")?;
        Ok(result.removed)
    }

    async fn call<T: DeserializeOwned>(&self, command: &str, body: Value) -> Result<T, Error> {
        let mut response = self
            .http
            .post(format!("{}/ticketz/{command}", self.url))
            .json(&body)
            .send()
            .await
            .map_err(transport)?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport)? {
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                return Err(Error::Protocol("HTTP response exceeds 32 MiB".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "hatch returned HTTP {status} for {command}"
            )));
        }
        let envelope: Envelope = serde_json::from_slice(&bytes).map_err(protocol)?;
        if envelope.truncated {
            return Err(Error::Protocol(
                "hatch truncated the result; increase the service output limit".into(),
            ));
        }
        if envelope.exit_code != Some(0) {
            return Err(Error::Command {
                status: envelope.exit_code,
                message: envelope.stderr,
            });
        }
        serde_json::from_str(&envelope.stdout).map_err(protocol)
    }
}

fn version(actual: &str, expected: &str) -> Result<(), Error> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::Protocol(format!(
            "expected {expected}, received {actual}"
        )))
    }
}
fn transport(e: reqwest::Error) -> Error {
    Error::Transport(helpers::http::error_chain(&e))
}
fn protocol(e: serde_json::Error) -> Error {
    Error::Protocol(e.to_string())
}

/// Why the mirror call failed. A failed call never becomes an empty snapshot.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Connection, TLS or timeout failure.
    #[error("cannot reach the ticket mirror: {0}")]
    Transport(String),
    /// Invalid or incompatible HTTP/record response.
    #[error("invalid ticket mirror response: {0}")]
    Protocol(String),
    /// Hatch ran ticketz, which failed or ended without an exit status.
    #[error("ticketz exited with {status:?}: {message}")]
    Command {
        /// Process status, absent if killed.
        status: Option<i32>,
        /// Tool diagnostic, never a supplied credential.
        message: String,
    },
}

#[derive(Deserialize)]
struct Envelope {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    truncated: bool,
}
#[derive(Deserialize)]
struct WireSnapshot {
    version: String,
    issues: Vec<WireIssue>,
    boards: Vec<WireBoard>,
}
#[derive(Deserialize)]
struct WireIssue {
    version: String,
    repo: String,
    number: u64,
    title: String,
    url: String,
    state: String,
    author: Option<String>,
    created_at: String,
    updated_at: String,
    body: String,
    labels: Vec<String>,
    labels_complete: bool,
    label_events: Vec<WireEvent>,
    label_events_complete: bool,
}
#[derive(Deserialize)]
struct WireEvent {
    label: String,
    actor: Option<String>,
    at: String,
}
#[derive(Deserialize)]
struct WireBoard {
    version: String,
    repo: String,
    issues: usize,
    cursor: Option<String>,
    last_sync: String,
    last_full_sync: Option<String>,
    #[serde(default)]
    actors: BTreeMap<String, WireAccess>,
    board_files: Option<WireFiles>,
}
#[derive(Deserialize)]
struct WireFiles {
    branch: String,
    commit: String,
    paths: Vec<String>,
    complete: bool,
}
#[derive(Deserialize)]
struct WireAccess {
    permission: Option<String>,
    role: Option<String>,
    permission_unknown: Option<String>,
    observed_at: String,
}
impl WireAccess {
    fn into_access(self, login: String) -> Result<ActorAccess, Error> {
        let access = match (self.permission, self.permission_unknown) {
            (Some(permission), None) => Access::Known {
                permission,
                role: self.role,
            },
            (None, Some(why)) if self.role.is_none() => Access::Unknown(why),
            _ => {
                return Err(Error::Protocol(
                    "actor access must be either known or unknown".into(),
                ));
            }
        };
        Ok(ActorAccess {
            login,
            access,
            observed_at: self.observed_at,
        })
    }
}
