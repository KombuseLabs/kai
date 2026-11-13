//! An issue read as a ticket: the body as the kai ticket form.
//!
//! The store keeps issues as GitHub said them; this is where kai reads
//! them. So a change to how the form is read takes effect on the next
//! command, without a sync.
//!
//! ```
//! use types::{Issue, IssueId, IssueState};
//!
//! let issue = Issue {
//!     id: IssueId { repo: "example-org/example-board".parse()?, number: 7 },
//!     title: "Invented ticket".to_owned(),
//!     url: "https://github.com/example-org/example-board/issues/7".to_owned(),
//!     state: IssueState::Open,
//!     author: None,
//!     created_at: "2026-01-02T03:04:05Z".to_owned(),
//!     updated_at: "2026-01-02T03:04:05Z".to_owned(),
//!     body: "### Instruction\n\nCount the widgets.\n\n### Check\n\n_No response_\n".to_owned(),
//!     labels: vec!["kai".to_owned()],
//!     labels_complete: true,
//!     label_events: vec![],
//!     label_events_complete: true,
//! };
//! let ticket = engine::ticket::from_issue(&issue);
//! let form = ticket.form.unwrap();
//! assert_eq!(form.instruction.as_deref(), Some("Count the widgets."));
//! assert_eq!(form.check, None);
//! # Ok::<(), types::ParseRepoNameError>(())
//! ```

mod form;

use types::{Issue, Ticket};

/// The ticket `issue` is: its fields, and its body read as the kai ticket
/// form, `None` when the body is not that form.
pub fn from_issue(issue: &Issue) -> Ticket {
    Ticket {
        issue: issue.id.clone(),
        title: issue.title.clone(),
        url: issue.url.clone(),
        state: issue.state,
        labels: issue.labels.clone(),
        labels_complete: issue.labels_complete,
        label_events: issue.label_events.clone(),
        label_events_complete: issue.label_events_complete,
        updated_at: issue.updated_at.clone(),
        form: form::parse(&issue.body),
    }
}
