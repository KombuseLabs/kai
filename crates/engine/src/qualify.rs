//! Static qualification: whether kai may use a ticket, judged only from
//! what GitHub says about it. No repository, no VM, no model, no network:
//! a pure function of the [`Ticket`]. The rules, and how they add up to a
//! verdict, as diagrams: `docs/qualify.md` in the repository.
//!
//! What static qualification cannot know is reported as not checked, never
//! as met, so that no ticket is qualified on a rule nobody applied. Today
//! that is the trust gate (who put the `kai` label on) and whether the
//! repository has the commit and the hidden tests.
//!
//! ```
//! use engine::qualify::qualify;
//! use types::{Form, IssueId, IssueState, Rule, Ticket, Verdict};
//!
//! let ticket = Ticket {
//!     issue: IssueId { repo: "example-org/example-board".parse()?, number: 7 },
//!     title: "Invented ticket".to_owned(),
//!     url: "https://github.com/example-org/example-board/issues/7".to_owned(),
//!     state: IssueState::Open,
//!     labels: vec![],
//!     labels_complete: true,
//!     label_events: vec![],
//!     label_events_complete: true,
//!     updated_at: "2026-01-02T03:04:05Z".to_owned(),
//!     form: Some(Form::default()),
//! };
//! let qualification = qualify(&ticket);
//! assert_eq!(qualification.verdict(), Verdict::Refused);
//! let failed: Vec<Rule> = qualification
//!     .findings
//!     .iter()
//!     .filter(|f| matches!(f.outcome, types::Outcome::Fail(_)))
//!     .map(|f| f.rule)
//!     .collect();
//! assert_eq!(failed, [Rule::Label, Rule::Instruction, Rule::Repo, Rule::Ref, Rule::HiddenTests]);
//! # Ok::<(), types::ParseRepoNameError>(())
//! ```

use types::{Finding, Form, IssueState, Qualification, RepoName, Rule, Ticket};

/// The label that offers an issue to kai.
pub const LABEL: &str = "kai";

/// Applies the static rules to `ticket`. The rules on the form's fields
/// apply only when the body is the form.
pub fn qualify(ticket: &Ticket) -> Qualification {
    let mut findings = vec![
        label(ticket),
        Finding::not_checked(
            Rule::TrustGate,
            format!("who put the `{LABEL}` label on is not checked yet"),
        ),
        open(ticket),
    ];
    match &ticket.form {
        None => findings.push(Finding::fail(
            Rule::Form,
            "the issue's body is not the kai ticket form",
        )),
        Some(form) => {
            findings.push(Finding::pass(Rule::Form));
            findings.extend(form_rules(form));
        }
    }
    let qualification = Qualification { findings };
    tracing::debug!(issue = %ticket.issue, verdict = %qualification.verdict(), "qualified statically");
    qualification
}

fn label(ticket: &Ticket) -> Finding {
    if ticket.labels.iter().any(|label| label == LABEL) {
        Finding::pass(Rule::Label)
    } else if !ticket.labels_complete {
        Finding::not_checked(
            Rule::Label,
            format!(
                "the issue has more labels than were fetched, and `{LABEL}` is not among those"
            ),
        )
    } else {
        Finding::fail(Rule::Label, format!("the issue has no `{LABEL}` label"))
    }
}

fn open(ticket: &Ticket) -> Finding {
    match ticket.state {
        IssueState::Open => Finding::pass(Rule::Open),
        IssueState::Closed => Finding::fail(Rule::Open, "the issue is closed"),
    }
}

fn form_rules(form: &Form) -> Vec<Finding> {
    let instruction = form.instruction.as_deref().unwrap_or("");
    vec![
        if instruction.trim().is_empty() {
            Finding::fail(Rule::Instruction, "the instruction is empty")
        } else {
            Finding::pass(Rule::Instruction)
        },
        if has_code_block(instruction) {
            Finding::fail(
                Rule::NoCodeBlock,
                "the instruction holds a code block, which may give the solution away",
            )
        } else {
            Finding::pass(Rule::NoCodeBlock)
        },
        match form.repo.as_deref() {
            None => Finding::fail(Rule::Repo, "the ticket names no repository"),
            Some(repo) => match repo.parse::<RepoName>() {
                Ok(_) => Finding::pass(Rule::Repo),
                Err(err) => Finding::fail(Rule::Repo, err.to_string()),
            },
        },
        match form.base_ref.as_deref() {
            None => Finding::fail(Rule::Ref, "the ticket names no ref"),
            Some(base_ref) if is_full_sha(base_ref) => Finding::pass(Rule::Ref),
            Some(base_ref) => Finding::fail(
                Rule::Ref,
                format!("`{base_ref}` is not a full commit sha (40 hex digits)"),
            ),
        },
        match form.check {
            Some(_) => Finding::pass(Rule::Check),
            None => Finding::not_checked(
                Rule::Check,
                "the ticket names no check, and the ecosystem's default is not detected yet",
            ),
        },
        match form.hidden_tests {
            Some(_) => Finding::pass(Rule::HiddenTests),
            None => Finding::fail(
                Rule::HiddenTests,
                "the ticket names no hidden tests, so nothing can tell a solution from none",
            ),
        },
        Finding::not_checked(
            Rule::AtRef,
            "whether the repository has the commit and the hidden tests is not checked yet",
        ),
    ]
}

/// Whether `text` holds a fenced code block: a line that starts, after
/// indentation, with ```` ``` ```` or `~~~`. Inline code does not count.
fn has_code_block(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("```") || line.starts_with("~~~")
    })
}

/// Whether `text` is a full SHA-1 commit id.
fn is_full_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit())
}
