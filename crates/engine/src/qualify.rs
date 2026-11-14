//! Static qualification: whether kai may use a ticket, judged from what
//! GitHub says about it and what kai found before. No repository, no VM, no
//! model, no network: a pure function of the [`Ticket`] and its [`Facts`].
//! The rules, and how they add up to a verdict, as diagrams:
//! `docs/qualify.md` in the repository.
//!
//! What qualification cannot know is reported as not checked, never as
//! met, so that no ticket is qualified on a rule nobody applied.
//!
//! - The trust gate reads the board's [`ActorAccess`]es, what GitHub said
//!   of each label actor's access (`kai github sync --actor-permissions`):
//!   the label counts only when the account that put it on last has write
//!   access now.
//! - `at-ref` reads whether a fetch run checked the ticket's commit out
//!   (`kai ticket deps`), and the board's files (`kai github sync`): the
//!   hidden tests lie in the board, never in the code repository the agent
//!   works in, and must match a file there.
//!
//! ```
//! use engine::qualify::{Facts, qualify};
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
//! let qualification = qualify(&ticket, &Facts::default());
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

use types::{
    Access, ActorAccess, BoardFiles, Finding, Form, IssueState, Qualification, RepoName, Rule,
    Ticket,
};

/// The label that offers an issue to kai.
pub const LABEL: &str = "kai";

/// The roles that may offer an issue to kai: those that can write to the
/// board.
const WRITERS: [&str; 3] = ["admin", "maintain", "write"];

/// What kai knows of a ticket beside the ticket itself. What it does not
/// know leaves the rules that need it not checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// What GitHub said of the access of the label actors of the ticket's
    /// board.
    pub access: Vec<ActorAccess>,
    /// The board's files as GitHub listed them last, or `None` when they
    /// were never listed.
    pub board_files: Option<BoardFiles>,
    /// Whether a fetch run checked out the ticket's commit of its
    /// repository (`engine::deps::fetched`).
    pub commit_fetched: bool,
}

/// Applies the rules to `ticket`, with what kai knows beside it. The rules
/// on the form's fields apply only when the body is the form.
pub fn qualify(ticket: &Ticket, facts: &Facts) -> Qualification {
    let mut findings = vec![
        label(ticket),
        trust_gate(ticket, &facts.access),
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
            findings.push(at_ref(ticket, form, facts));
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

/// Whether the account that put the `kai` label on last may write to the
/// board, as GitHub said when asked. Not checked when that cannot be told:
/// the later labeled events were not fetched, or the account's access was
/// not asked or not told.
fn trust_gate(ticket: &Ticket, access: &[ActorAccess]) -> Finding {
    let not_checked = |why: String| Finding::not_checked(Rule::TrustGate, why);
    if !ticket.labels.iter().any(|label| label == LABEL) {
        return not_checked(format!("the issue has no `{LABEL}` label to check"));
    }
    if !ticket.label_events_complete {
        return not_checked(format!(
            "not every labeled event was fetched, so who put `{LABEL}` on last is not known"
        ));
    }
    let Some(event) = ticket.label_events.iter().rev().find(|e| e.label == LABEL) else {
        return not_checked(format!("no event that put `{LABEL}` on was fetched"));
    };
    let Some(login) = &event.actor else {
        return Finding::fail(
            Rule::TrustGate,
            format!("`{LABEL}` was put on by an account that no longer exists"),
        );
    };
    match access.iter().find(|actor| &actor.login == login) {
        None => not_checked(format!(
            "the access of {login}, who put `{LABEL}` on, was not asked; \
             `kai github sync --actor-permissions` asks it"
        )),
        Some(ActorAccess {
            access: Access::Unknown(why),
            ..
        }) => not_checked(format!(
            "GitHub did not tell the access of {login}, who put `{LABEL}` on: {why}"
        )),
        Some(ActorAccess {
            access: Access::Known { permission, role },
            observed_at,
            ..
        }) => {
            let role = role.as_deref().unwrap_or(permission);
            if WRITERS.contains(&role) {
                Finding::pass(Rule::TrustGate)
            } else {
                Finding::fail(
                    Rule::TrustGate,
                    format!(
                        "`{LABEL}` was put on by {login}, whose access is `{role}`, \
                         not write (asked {observed_at})"
                    ),
                )
            }
        }
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
    ]
}

/// Whether the repository has the ticket's commit, and the board the hidden
/// tests: a fetch run checked the commit out, and a file of the board
/// matches the hidden tests' path.
fn at_ref(ticket: &Ticket, form: &Form, facts: &Facts) -> Finding {
    let not_checked = |why: String| Finding::not_checked(Rule::AtRef, why);
    let (Some(repo), Some(_), Some(hidden)) = (
        form.repo
            .as_deref()
            .and_then(|repo| repo.parse::<RepoName>().ok()),
        form.base_ref.as_deref().filter(|base| is_full_sha(base)),
        form.hidden_tests.as_deref(),
    ) else {
        return not_checked(
            "the ticket names no repository, ref or hidden tests to look for".to_owned(),
        );
    };
    if repo == ticket.issue.repo {
        return Finding::fail(
            Rule::AtRef,
            "the code repository is the board itself, so the hidden tests would lie in the tree the agent works in",
        );
    }
    let hidden = if hidden.starts_with("https://") || hidden.starts_with("http://") {
        Some(not_checked(
            "hidden tests from a pull request are not read yet".to_owned(),
        ))
    } else {
        match &facts.board_files {
            None => Some(not_checked(
                "the board's files were not listed; `kai github sync` lists them".to_owned(),
            )),
            Some(files) if files.paths.iter().any(|path| matches(hidden, path)) => None,
            Some(files) if !files.complete => Some(not_checked(format!(
                "no file of the board matches `{hidden}` among those GitHub listed, and it cut the listing short"
            ))),
            Some(files) => {
                return Finding::fail(
                    Rule::AtRef,
                    format!(
                        "no file of the board at {} ({}) matches the hidden tests `{hidden}`",
                        files.branch,
                        &files.commit[..files.commit.len().min(12)]
                    ),
                );
            }
        }
    };
    if !facts.commit_fetched {
        return not_checked(format!(
            "whether {repo} has the commit is not known: no fetch run checked it out; `kai ticket deps {}` does",
            ticket.issue.number
        ));
    }
    hidden.unwrap_or_else(|| Finding::pass(Rule::AtRef))
}

/// Whether the file `path` of the board is one of the hidden tests
/// `pattern` names: the file itself or a file under it when it holds no
/// wildcard; else a glob in which `*` and `?` stay within one part of the
/// path and `**` stands for any number of parts.
fn matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern
        .trim()
        .trim_start_matches("./")
        .trim_start_matches('/');
    if !pattern.contains(['*', '?']) {
        let pattern = pattern.trim_end_matches('/');
        return path == pattern
            || path
                .strip_prefix(pattern)
                .is_some_and(|rest| rest.starts_with('/'));
    }
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    parts_match(&pattern, &path)
}

/// Whether the parts of a path match those of a glob.
fn parts_match(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| parts_match(rest, &path[skip..])),
        Some((first, rest)) => path.split_first().is_some_and(|(part, others)| {
            part_matches(first.as_bytes(), part.as_bytes()) && parts_match(rest, others)
        }),
    }
}

/// Whether one part of a path matches one part of a glob.
fn part_matches(pattern: &[u8], part: &[u8]) -> bool {
    match pattern.split_first() {
        None => part.is_empty(),
        Some((b'*', rest)) => (0..=part.len()).any(|skip| part_matches(rest, &part[skip..])),
        Some((b'?', rest)) => !part.is_empty() && part_matches(rest, &part[1..]),
        Some((c, rest)) => part.first() == Some(c) && part_matches(rest, &part[1..]),
    }
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

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn a_path_names_the_file_or_what_lies_under_it() {
        assert!(matches("tests/hidden/count.rs", "tests/hidden/count.rs"));
        assert!(matches("./tests/hidden/", "tests/hidden/count.rs"));
        assert!(matches("tests/hidden", "tests/hidden/deeper/count.rs"));
        assert!(!matches("tests/hidden", "tests/hidden-not/count.rs"));
        assert!(!matches(
            "tests/hidden/count.rs",
            "tests/hidden/count.rs.bak"
        ));
    }

    #[test]
    fn a_glob_keeps_its_wildcards_within_a_part_but_two_stars_cross() {
        assert!(matches("tests/hidden/*.rs", "tests/hidden/count.rs"));
        assert!(!matches(
            "tests/hidden/*.rs",
            "tests/hidden/deeper/count.rs"
        ));
        assert!(matches(
            "tests/hidden/**/*.rs",
            "tests/hidden/deeper/count.rs"
        ));
        assert!(matches("tests/hidden/**/*.rs", "tests/hidden/count.rs"));
        assert!(matches("tests/hidden/c?unt.rs", "tests/hidden/count.rs"));
        assert!(!matches("tests/*/count.rs", "tests/count.rs"));
    }
}
