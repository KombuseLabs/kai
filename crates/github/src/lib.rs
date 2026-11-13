//! Reads a board's issues from GitHub into [`Issue`]s, as GitHub said
//! them. One way: nothing here writes to GitHub.
//!
//! - **One GraphQL query per page of 50 issues**: each issue with its
//!   author, its body, its labels, and who put each label on (the
//!   timeline's labeled events), oldest change first. The body is kept
//!   whole; reading it as the kai ticket form is `engine::ticket`'s. More
//!   than 100 labels or labeled events are not fetched, and the issue says
//!   so ([`Issue::labels_complete`], [`Issue::label_events_complete`]).
//! - **From a cursor**: [`Client::fetch_board`] takes the time the last
//!   sync recorded and asks only for issues changed since then, and says
//!   where the next sync starts.
//! - **The token** is a [`SecretString`]: it never shows in `Debug` output
//!   or a log line, and it is sent only to the API the client was made for.
//!   Redirects are not followed.
//! - **Async**, on reqwest and so on tokio. The program that calls
//!   [`Client::fetch_board`] runs the runtime; this crate never starts one.

use std::time::Duration;

use helpers::http;
use secrecy::ExposeSecret as _;
/// The token's type, so that callers need not depend on `secrecy`.
pub use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use types::{Issue, IssueId, IssueState, LabelEvent, RepoName};

/// The API of github.com.
pub const API: &str = "https://api.github.com";

/// GitHub refuses requests without a user agent.
const USER_AGENT: &str = concat!("kai/", env!("CARGO_PKG_VERSION"));

/// Issues asked for per request.
const PAGE: u32 = 50;

/// The board's issues, oldest change first: author, body, labels, and
/// every time a label was put on. Labels and events beyond 100 are not
/// fetched; `hasNextPage` says whether there are more.
const BOARD_QUERY: &str = r#"
query ($owner: String!, $name: String!, $since: DateTime, $after: String, $page: Int!) {
  repository(owner: $owner, name: $name) {
    issues(first: $page, after: $after, filterBy: {since: $since},
           orderBy: {field: UPDATED_AT, direction: ASC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number
        title
        url
        state
        author { login }
        body
        createdAt
        updatedAt
        labels(first: 100) { pageInfo { hasNextPage } nodes { name } }
        timelineItems(itemTypes: [LABELED_EVENT], first: 100) {
          pageInfo { hasNextPage }
          nodes { ... on LabeledEvent { createdAt actor { login } label { name } } }
        }
      }
    }
  }
  rateLimit { cost remaining resetAt }
}
"#;

/// A connection to GitHub's API with one token.
#[derive(Debug)]
pub struct Client {
    http: reqwest::Client,
    /// Where GraphQL requests go: `<api>/graphql`.
    graphql: String,
    token: SecretString,
}

/// What one sync fetched.
#[derive(Debug)]
pub struct Fetched {
    /// The issues changed since the cursor, oldest change first.
    pub issues: Vec<Issue>,
    /// Where the next sync starts: the latest change among the tickets, or
    /// the cursor given when nothing changed; `None` when there was neither.
    pub since: Option<String>,
}

impl Client {
    /// A client for the API at `api` (normally [`API`]) that authenticates
    /// with `token`. The API must be `https://`, except on this machine
    /// (`http://127.0.0.1`, `http://localhost`, `http://[::1]`), so that the
    /// token never crosses a network in the clear.
    pub fn new(token: SecretString, api: &str) -> Result<Client, Error> {
        if !http::allowed_url(api) {
            return Err(Error::Api(api.to_owned()));
        }
        let http = http::builder(USER_AGENT)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| Error::Transport(http::error_chain(&err)))?;
        Ok(Client {
            http,
            graphql: format!("{}/graphql", api.trim_end_matches('/')),
            token,
        })
    }

    /// The issues of `board` that changed at or after `since` (RFC 3339),
    /// or all of them when `since` is `None`.
    pub async fn fetch_board(
        &self,
        board: &RepoName,
        since: Option<&str>,
    ) -> Result<Fetched, Error> {
        let mut issues = Vec::new();
        let mut after = None;
        loop {
            let request = Request {
                query: BOARD_QUERY,
                variables: Variables {
                    owner: board.owner(),
                    name: board.name(),
                    since,
                    after: after.as_deref(),
                    page: PAGE,
                },
            };
            let data: wire::Data = self.query(&request).await?;
            if let Some(limit) = &data.rate_limit {
                tracing::debug!(cost = limit.cost, remaining = limit.remaining, reset_at = %limit.reset_at, "GitHub's rate limit");
            }
            let repository = data
                .repository
                .ok_or_else(|| Error::NoBoard(board.clone()))?;
            let page = repository.issues;
            tracing::debug!(%board, issues = page.nodes.len(), "fetched a page of issues");
            for node in page.nodes.into_iter().flatten() {
                issues.push(node.into_issue(board));
            }
            match (page.page_info.has_next_page, page.page_info.end_cursor) {
                (true, Some(cursor)) => after = Some(cursor),
                _ => break,
            }
        }
        let since = issues
            .iter()
            .map(|issue| issue.updated_at.as_str())
            .chain(since)
            .max()
            .map(str::to_owned);
        Ok(Fetched { issues, since })
    }

    /// Sends one GraphQL request and returns its `data`. An HTTP error, or
    /// an answer with `errors`, is an error, even if it also holds data.
    async fn query<T: DeserializeOwned>(&self, request: &Request<'_>) -> Result<T, Error> {
        let response = self
            .http
            .post(&self.graphql)
            .bearer_auth(self.token.expose_secret())
            .json(request)
            .send()
            .await
            .map_err(|err| Error::Transport(http::error_chain(&err)))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|err| Error::Transport(http::error_chain(&err)))?;
        if !status.is_success() {
            let message = match serde_json::from_slice::<wire::Message>(&body) {
                Ok(answer) => answer.message,
                Err(_) => String::from_utf8_lossy(&body).chars().take(200).collect(),
            };
            return Err(Error::Status {
                status: status.as_u16(),
                message,
            });
        }
        let answer: wire::Answer<T> =
            serde_json::from_slice(&body).map_err(|err| Error::Answer(err.to_string()))?;
        if !answer.errors.is_empty() {
            return Err(Error::Query(
                answer.errors.into_iter().map(|err| err.message).collect(),
            ));
        }
        answer
            .data
            .ok_or_else(|| Error::Answer("neither data nor errors".to_owned()))
    }
}

/// The token `gh auth token` prints: that of the GitHub CLI's login on
/// this machine.
pub fn token_from_gh() -> Result<SecretString, Error> {
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|err| Error::NoToken(format!("cannot run gh: {err}")))?;
    if !output.status.success() {
        return Err(Error::NoToken(format!(
            "`gh auth token` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let token = String::from_utf8(output.stdout)
        .map_err(|_| Error::NoToken("`gh auth token` printed no text".to_owned()))?;
    let token = token.trim();
    if token.is_empty() {
        return Err(Error::NoToken("`gh auth token` printed nothing".to_owned()));
    }
    Ok(SecretString::from(token.to_owned()))
}

#[derive(Serialize)]
struct Request<'a> {
    query: &'a str,
    variables: Variables<'a>,
}

/// `since` and `after` are sent as `null` when there is none.
#[derive(Serialize)]
struct Variables<'a> {
    owner: &'a str,
    name: &'a str,
    since: Option<&'a str>,
    after: Option<&'a str>,
    page: u32,
}

/// GitHub's answer, as far as kai reads it. kai's own structs, not
/// a client library's models, so that the fixtures are plain JSON.
mod wire {
    use super::*;

    /// A GraphQL answer: `data`, `errors`, or both.
    #[derive(Deserialize)]
    pub(super) struct Answer<T> {
        pub(super) data: Option<T>,
        #[serde(default)]
        pub(super) errors: Vec<Message>,
    }

    /// A GraphQL error, or the body of an HTTP error: GitHub says what is
    /// wrong in `message`.
    #[derive(Deserialize)]
    pub(super) struct Message {
        pub(super) message: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct Data {
        pub(super) repository: Option<Repository>,
        pub(super) rate_limit: Option<RateLimit>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct RateLimit {
        pub(super) cost: u32,
        pub(super) remaining: u32,
        pub(super) reset_at: String,
    }

    #[derive(Deserialize)]
    pub(super) struct Repository {
        pub(super) issues: Connection<Issue>,
    }

    /// A page of a list. GraphQL may put `null` for an item it could not
    /// resolve; such an item is skipped.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct Connection<T> {
        pub(super) page_info: PageInfo,
        pub(super) nodes: Vec<Option<T>>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct PageInfo {
        pub(super) has_next_page: bool,
        #[serde(default)]
        pub(super) end_cursor: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct Issue {
        pub(super) number: u64,
        pub(super) title: String,
        pub(super) url: String,
        pub(super) state: String,
        /// `null` for a deleted account.
        pub(super) author: Option<Actor>,
        pub(super) body: String,
        pub(super) created_at: String,
        pub(super) updated_at: String,
        pub(super) labels: Option<Connection<Label>>,
        pub(super) timeline_items: Connection<LabeledEvent>,
    }

    #[derive(Deserialize)]
    pub(super) struct Label {
        pub(super) name: String,
    }

    /// Every field is optional: an item of another kind comes as `{}`.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct LabeledEvent {
        #[serde(default)]
        pub(super) created_at: Option<String>,
        #[serde(default)]
        pub(super) actor: Option<Actor>,
        #[serde(default)]
        pub(super) label: Option<Label>,
    }

    #[derive(Deserialize)]
    pub(super) struct Actor {
        pub(super) login: String,
    }

    impl Issue {
        pub(super) fn into_issue(self, board: &RepoName) -> super::Issue {
            let id = IssueId {
                repo: board.clone(),
                number: self.number,
            };
            let (labels, labels_complete) = match self.labels {
                Some(labels) => (
                    labels
                        .nodes
                        .into_iter()
                        .flatten()
                        .map(|label| label.name)
                        .collect(),
                    !labels.page_info.has_next_page,
                ),
                None => (Vec::new(), true),
            };
            let label_events_complete = !self.timeline_items.page_info.has_next_page;
            let label_events = self
                .timeline_items
                .nodes
                .into_iter()
                .flatten()
                .filter_map(|event| {
                    Some(LabelEvent {
                        label: event.label?.name,
                        actor: event.actor.map(|actor| actor.login),
                        at: event.created_at?,
                    })
                })
                .collect();
            if !labels_complete || !label_events_complete {
                tracing::debug!(%id, labels_complete, label_events_complete, "not every label or labeled event was fetched");
            }
            super::Issue {
                id,
                title: self.title,
                url: self.url,
                state: if self.state == "OPEN" {
                    IssueState::Open
                } else {
                    IssueState::Closed
                },
                author: self.author.map(|author| author.login),
                created_at: self.created_at,
                updated_at: self.updated_at,
                body: self.body,
                labels,
                labels_complete,
                label_events,
                label_events_complete,
            }
        }
    }
}

/// Why GitHub could not be asked, or what it refused.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The API's address is not a URL, or is plain HTTP to another machine.
    #[error("`{0}` is not an https:// URL (plain http:// is taken only for this machine)")]
    Api(String),
    /// No token: neither given nor to be had from `gh`.
    #[error("no GitHub token: {0}")]
    NoToken(String),
    /// GitHub answered with an HTTP error, such as 401 for a bad token or
    /// 403 for a rate limit.
    #[error("GitHub answered {status}: {message}")]
    Status {
        /// The HTTP status.
        status: u16,
        /// GitHub's message.
        message: String,
    },
    /// GitHub refused the query, for instance because the board does not
    /// exist or the token cannot read it.
    #[error("GitHub refused the query: {}", .0.join("; "))]
    Query(Vec<String>),
    /// GitHub answered without the board and without saying why.
    #[error("GitHub has no repository {0}, or the token cannot read it")]
    NoBoard(RepoName),
    /// The request did not get an answer: the network, TLS, a timeout.
    #[error("cannot talk to GitHub: {0}")]
    Transport(String),
    /// GitHub answered, but not in the shape kai reads.
    #[error("GitHub's answer is not what kai reads: {0}")]
    Answer(String),
}
