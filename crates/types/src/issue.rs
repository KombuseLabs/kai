//! An issue as GitHub said it: what a sync fetches and the store keeps.

use crate::{IssueId, IssueState, LabelEvent};

/// An issue as GitHub said it, and nothing kai read into it: the body is
/// kept whole, and whether it is the kai ticket form is decided when a
/// [`Ticket`](crate::Ticket) is made from it. What the store keeps, so
/// that a change to how kai reads an issue needs no sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// Which issue.
    pub id: IssueId,
    /// The issue's title.
    pub title: String,
    /// The issue's page on GitHub.
    pub url: String,
    /// Whether the issue is open or closed.
    pub state: IssueState,
    /// The login of the account that opened the issue, or `None` when
    /// GitHub does not say (a deleted account).
    pub author: Option<String>,
    /// When the issue was opened, as GitHub wrote it (RFC 3339).
    pub created_at: String,
    /// When the issue last changed, as GitHub wrote it (RFC 3339).
    pub updated_at: String,
    /// The issue's body as its author wrote it; empty when there is none.
    pub body: String,
    /// The labels on the issue now, in GitHub's order.
    pub labels: Vec<String>,
    /// Whether [`labels`](Self::labels) are all of them. A sync fetches
    /// at most 100.
    pub labels_complete: bool,
    /// Each time a label was put on the issue, oldest first, including
    /// labels that were removed again.
    pub label_events: Vec<LabelEvent>,
    /// Whether [`label_events`](Self::label_events) are all of them. A
    /// sync fetches the oldest 100, so when this is `false` the latest are
    /// the ones missing.
    pub label_events_complete: bool,
}
