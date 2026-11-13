//! Static qualification on hand-written, invented tickets.

use engine::qualify::qualify;
use types::{
    Access, ActorAccess, Form, IssueId, IssueState, LabelEvent, Outcome, Rule, Ticket, Verdict,
};

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
        labels_complete: true,
        label_events: vec![
            LabelEvent {
                label: "kai".to_owned(),
                actor: Some("example-maintainer".to_owned()),
                at: "2026-01-02T03:00:00Z".to_owned(),
            },
            LabelEvent {
                label: "bug".to_owned(),
                actor: Some("example-triager".to_owned()),
                at: "2026-01-02T03:01:00Z".to_owned(),
            },
        ],
        label_events_complete: true,
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

/// What GitHub said of the board's label actors: the maintainer may
/// write, the triager may not.
fn access() -> Vec<ActorAccess> {
    let actor = |login: &str, role: &str| ActorAccess {
        login: login.to_owned(),
        access: Access::Known {
            permission: if role == "triage" { "read" } else { "write" }.to_owned(),
            role: Some(role.to_owned()),
        },
        observed_at: "2026-01-03T00:00:00Z".to_owned(),
    };
    vec![
        actor("example-maintainer", "maintain"),
        actor("example-triager", "triage"),
    ]
}

/// The outcome of `rule`, which must have been applied once.
fn outcome(ticket: &Ticket, rule: Rule) -> Outcome {
    outcome_with(ticket, &access(), rule)
}

/// The outcome of `rule` with `access`.
fn outcome_with(ticket: &Ticket, access: &[ActorAccess], rule: Rule) -> Outcome {
    let findings = qualify(ticket, access).findings;
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
    let qualification = qualify(&ticket, &access());
    assert_eq!(qualification.verdict(), Verdict::Undecided);
    let not_checked: Vec<Rule> = qualification
        .findings
        .iter()
        .filter(|f| !matches!(f.outcome, Outcome::Pass))
        .map(|f| f.rule)
        .collect();
    assert_eq!(not_checked, [Rule::AtRef]);
}

#[test]
fn the_gate_passes_for_every_role_that_can_write_and_by_permission_without_one() {
    for (permission, role) in [
        ("admin", Some("admin")),
        ("write", Some("maintain")),
        ("write", Some("write")),
        ("write", None),
    ] {
        let access = [ActorAccess {
            login: "example-maintainer".to_owned(),
            access: Access::Known {
                permission: permission.to_owned(),
                role: role.map(str::to_owned),
            },
            observed_at: "2026-01-03T00:00:00Z".to_owned(),
        }];
        assert_eq!(
            outcome_with(&good(), &access, Rule::TrustGate),
            Outcome::Pass,
            "{permission} {role:?}"
        );
    }
}

#[test]
fn the_label_put_on_last_by_someone_who_cannot_write_is_refused() {
    let mut ticket = good();
    // The maintainer put it on first; the triager took it off and put it
    // on again.
    ticket.label_events.push(LabelEvent {
        label: "kai".to_owned(),
        actor: Some("example-triager".to_owned()),
        at: "2026-01-02T04:00:00Z".to_owned(),
    });
    match outcome(&ticket, Rule::TrustGate) {
        Outcome::Fail(why) => assert!(why.contains("example-triager") && why.contains("`triage`")),
        other => panic!("{other:?}"),
    }
    assert_eq!(qualify(&ticket, &access()).verdict(), Verdict::Refused);
}

#[test]
fn the_label_put_on_by_a_deleted_account_is_refused() {
    let mut ticket = good();
    ticket.label_events[0].actor = None;
    assert!(fails(&ticket, Rule::TrustGate));
}

#[test]
fn the_gate_is_not_checked_when_the_access_is_not_known() {
    let unknown = [ActorAccess {
        login: "example-maintainer".to_owned(),
        access: Access::Unknown("GitHub answered 404: invented".to_owned()),
        observed_at: "2026-01-03T00:00:00Z".to_owned(),
    }];
    let mut cut = good();
    cut.label_events_complete = false;
    let mut unlabelled = good();
    unlabelled.labels.retain(|label| label != "kai");
    let mut no_event = good();
    no_event.label_events.clear();
    for (ticket, access, what) in [
        (good(), &[][..], "not asked"),
        (good(), &unknown[..], "not told"),
        (cut, &access()[..], "later events not fetched"),
        (unlabelled, &access()[..], "no label"),
        (no_event, &access()[..], "no event"),
    ] {
        assert!(
            matches!(
                outcome_with(&ticket, access, Rule::TrustGate),
                Outcome::NotChecked(_)
            ),
            "{what}"
        );
    }
}

#[test]
fn without_the_kai_label_it_is_refused() {
    let mut ticket = good();
    ticket.labels.retain(|label| label != "kai");
    assert!(fails(&ticket, Rule::Label));
    assert_eq!(qualify(&ticket, &access()).verdict(), Verdict::Refused);
}

#[test]
fn without_the_kai_label_among_too_many_fetched_it_is_not_checked() {
    let mut ticket = good();
    ticket.labels.retain(|label| label != "kai");
    ticket.labels_complete = false;
    assert!(matches!(
        outcome(&ticket, Rule::Label),
        Outcome::NotChecked(_)
    ));
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
    let qualification = qualify(&ticket, &access());
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
