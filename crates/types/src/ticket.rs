//! A ticket: an issue on a board that asks for one piece of work.

use crate::IssueId;

/// An issue on a board, as kai fetched it. Whether kai may use it is what
/// qualifying it decides; a ticket is kept whether it qualifies or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    /// The issue the ticket was made from: where it came from.
    pub issue: IssueId,
    /// The issue's title.
    pub title: String,
    /// The issue's page on GitHub.
    pub url: String,
    /// Whether the issue is open or closed.
    pub state: IssueState,
    /// The labels on the issue now, in GitHub's order.
    pub labels: Vec<String>,
    /// Each time a label was put on the issue, oldest first, including
    /// labels that were removed again.
    pub label_events: Vec<LabelEvent>,
    /// When the issue last changed, as GitHub wrote it (RFC 3339).
    pub updated_at: String,
    /// The answers of the kai ticket form, or `None` when the issue's body
    /// is not that form.
    pub form: Option<Form>,
}

/// Whether an issue is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueState {
    /// The issue is open.
    Open,
    /// The issue is closed, for whatever reason.
    Closed,
}

/// A label being put on an issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelEvent {
    /// The label's name.
    pub label: String,
    /// The login of the account that put it there, or `None` when GitHub
    /// does not say (a deleted account).
    pub actor: Option<String>,
    /// When, as GitHub wrote it (RFC 3339).
    pub at: String,
}

/// The fields of the kai ticket form as the issue's author filled them in.
/// A field left empty is `None`. Nothing here is checked; that is what
/// qualifying does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    /// What the agent is asked to do.
    pub instruction: Option<String>,
    /// The code repository, meant as `owner/name`.
    pub repo: Option<String>,
    /// The commit the work starts from, meant as a full sha.
    pub base_ref: Option<String>,
    /// The command that decides pass or fail; `None` asks for the
    /// ecosystem's default.
    pub check: Option<String>,
    /// Where the tests the agent does not see are: a path glob in the
    /// repository at the ref, or a link to a pull request.
    pub hidden_tests: Option<String>,
}
