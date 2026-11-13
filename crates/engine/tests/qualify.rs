//! Static qualification on hand-written, invented tickets.

use engine::qualify::qualify;
use types::{Form, IssueId, IssueState, Outcome, Rule, Ticket, Verdict};

/// A ticket that meets every rule static qualification can check.
fn good() -> Ticket {
    Ticket {
        issue: IssueId {
            repo: "example-org/example-board".parse().unwrap(),
            number: 3,
        },
        title: "Invented: count the widgets".to_owned(),
        url: "https://github.com/example-org/example-board/issues/3".to_owned(),
        state: IssueState::Open,
        labels: vec!["bug".to_owned(), "kai".to_owned()],
        label_events: vec![],
        updated_at: "2026-01-02T03:04:05Z".to_owned(),
        form: Some(Form {
            instruction: Some("Make `widgets --count` print the number of widgets.".to_owned()),
            repo: Some("example-org/example-code".to_owned()),
            base_ref: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            check: Some("cargo test --locked".to_owned()),
            hidden_tests: Some("tests/hidden/count.rs".to_owned()),
        }),
    }
}

/// The outcome of `rule`, which must have been applied once.
fn outcome(ticket: &Ticket, rule: Rule) -> Outcome {
    let findings = qualify(ticket).findings;
    let mut matching = findings.iter().filter(|f| f.rule == rule);
    let finding = matching.next().expect("the rule was applied");
    assert!(matching.next().is_none(), "the rule was applied twice");
    finding.outcome.clone()
}

fn fails(ticket: &Ticket, rule: Rule) -> bool {
    matches!(outcome(ticket, rule), Outcome::Fail(_))
}

#[test]
fn a_good_ticket_fails_no_rule_but_is_undecided_on_what_is_not_built() {
    let ticket = good();
    let qualification = qualify(&ticket);
    assert_eq!(qualification.verdict(), Verdict::Undecided);
    let not_checked: Vec<Rule> = qualification
        .findings
        .iter()
        .filter(|f| !matches!(f.outcome, Outcome::Pass))
        .map(|f| f.rule)
        .collect();
    assert_eq!(not_checked, [Rule::TrustGate, Rule::AtRef]);
}

#[test]
fn without_the_kai_label_it_is_refused() {
    let mut ticket = good();
    ticket.labels.retain(|label| label != "kai");
    assert!(fails(&ticket, Rule::Label));
    assert_eq!(qualify(&ticket).verdict(), Verdict::Refused);
}

#[test]
fn a_closed_issue_is_refused() {
    let mut ticket = good();
    ticket.state = IssueState::Closed;
    assert!(fails(&ticket, Rule::Open));
}

#[test]
fn a_body_that_is_not_the_form_is_refused_and_its_fields_are_not_judged() {
    let mut ticket = good();
    ticket.form = None;
    let qualification = qualify(&ticket);
    assert!(fails(&ticket, Rule::Form));
    assert!(!qualification.findings.iter().any(|f| f.rule == Rule::Repo));
}

#[test]
fn an_empty_or_blank_instruction_is_refused() {
    for instruction in [None, Some("  \n ")] {
        let mut ticket = good();
        ticket.form.as_mut().unwrap().instruction = instruction.map(str::to_owned);
        assert!(fails(&ticket, Rule::Instruction));
    }
}

#[test]
fn a_code_block_in_the_instruction_is_refused_inline_code_is_not() {
    for fence in ["```", "~~~", "  ```rust"] {
        let mut ticket = good();
        ticket.form.as_mut().unwrap().instruction = Some(format!(
            "Change it to\n\n{fence}\nlet n = widgets.len();\n```\n"
        ));
        assert!(fails(&ticket, Rule::NoCodeBlock), "{fence}");
    }
    assert_eq!(outcome(&good(), Rule::NoCodeBlock), Outcome::Pass);
}

#[test]
fn the_repository_must_be_owner_slash_name() {
    for repo in [
        None,
        Some("example-code"),
        Some("a/b/c"),
        Some("example-org/.."),
    ] {
        let mut ticket = good();
        ticket.form.as_mut().unwrap().repo = repo.map(str::to_owned);
        assert!(fails(&ticket, Rule::Repo), "{repo:?}");
    }
}

#[test]
fn the_ref_must_be_a_full_sha() {
    for base_ref in [
        None,
        Some("main"),
        Some("0123456"),
        Some("z123456789abcdef0123456789abcdef01234567"),
    ] {
        let mut ticket = good();
        ticket.form.as_mut().unwrap().base_ref = base_ref.map(str::to_owned);
        assert!(fails(&ticket, Rule::Ref), "{base_ref:?}");
    }
}

#[test]
fn no_check_is_not_checked_rather_than_passed() {
    let mut ticket = good();
    ticket.form.as_mut().unwrap().check = None;
    assert!(matches!(
        outcome(&ticket, Rule::Check),
        Outcome::NotChecked(_)
    ));
}

#[test]
fn no_hidden_tests_is_refused() {
    let mut ticket = good();
    ticket.form.as_mut().unwrap().hidden_tests = None;
    assert!(fails(&ticket, Rule::HiddenTests));
}
