//! The checks on an invented crate (`tests/fixtures/check/crates/shapes`):
//! diagrams that hold pass, and each kind of drift fails at its line with a
//! message that names what differs. The diagrams are written here; each
//! negative case is the passing diagram with one change.

use std::path::{Path, PathBuf};

use diagrams::Finding;
use diagrams::check::Checker;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/check")
}

/// The findings for `diagram` put in a Markdown file, after two lines of
/// text, so that the block's first line is line 4.
fn check_at(root: &Path, diagram: &str) -> Vec<Finding> {
    Checker::new(root).check_markdown("doc.md", &format!("# Doc\n\n```mermaid\n{diagram}```\n"))
}

fn check(diagram: &str) -> Vec<Finding> {
    check_at(&fixture(), diagram)
}

/// The line in the Markdown file of the first line of `diagram` that
/// contains `needle`.
fn line_of(diagram: &str, needle: &str) -> usize {
    4 + diagram
        .lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("`{needle}` is not in the diagram"))
}

/// Asserts that the findings are exactly one per expectation, each at its
/// line and containing its text.
fn expect(findings: &[Finding], expected: &[(usize, &str)]) {
    let shown: Vec<String> = findings.iter().map(ToString::to_string).collect();
    assert_eq!(findings.len(), expected.len(), "{shown:#?}");
    for (line, text) in expected {
        assert!(
            findings
                .iter()
                .any(|f| f.line == *line && f.message.contains(text)),
            "no finding at line {line} containing `{text}`: {shown:#?}"
        );
    }
}

const CLASSES: &str = "%% checked against crates/shapes
classDiagram
    direction LR
    class Ticket {
        +String title
        +Vec~String~ labels
        +Option~Form~ form
        +Vec~Event~ events
        +State state
        -u32 secret
        +verdict(&self) Outcome
        ~close(&mut self)
        +new(title: String)$ Ticket
        +start(&mut self) bool
    }
    class Form {
        +Option~String~ repo
    }
    class Event {
        <<excerpt>>
        +String at
    }
    class State {
        <<enumeration>>
        Open
        Closed
    }
    class Outcome {
        <<enumeration>>
        Pass
        Fail(String)
        Ended(how: String, code: i32)
    }
    class Run {
        <<trait>>
        +start(&mut self) bool*
        +stop(&mut self)
    }
    class qualify {
        <<function>>
        +qualify(ticket: &Ticket) Outcome
    }
    class Fence {
        <<process>>
        fence proxy --listen
    }
    Ticket \"1\" *-- \"0..1\" Form
    Ticket \"1\" *-- \"*\" Event
    Ticket --> State
    Ticket ..|> Run
    Ticket ..> Fence : starts
    qualify ..> Ticket : reads
    note for Outcome \"what qualifying found\"
";

/// The passing diagram with `from` replaced by `to`, once.
fn changed(from: &str, to: &str) -> String {
    assert!(CLASSES.contains(from), "`{from}` is not in the diagram");
    CLASSES.replacen(from, to, 1)
}

#[test]
fn a_class_diagram_that_holds_passes() {
    expect(&check(CLASSES), &[]);
}

#[test]
fn fields_are_checked_by_name_type_and_visibility() {
    let renamed = changed("+String title", "+String name");
    expect(
        &check(&renamed),
        &[
            (
                line_of(&renamed, "+String name"),
                "shows the field `name`, which `Ticket` does not have",
            ),
            (
                line_of(&renamed, "class Ticket"),
                "does not show the field `pub title: String` (crates/shapes/src/lib.rs:8)",
            ),
        ],
    );
    let retyped = changed("+Vec~String~ labels", "+Vec~u8~ labels");
    expect(
        &check(&retyped),
        &[(
            line_of(&retyped, "labels"),
            "shows `labels: Vec<u8>`; the code has `labels: Vec<String>` (crates/shapes/src/lib.rs:9)",
        )],
    );
    let published = changed("-u32 secret", "+u32 secret");
    expect(
        &check(&published),
        &[(
            line_of(&published, "secret"),
            "shows the field `secret` as `+`; the code makes it `-`",
        )],
    );
    let missing = changed("        +String title\n", "");
    expect(
        &check(&missing),
        &[(
            line_of(&missing, "class Ticket"),
            "does not show the field `pub title: String`",
        )],
    );
}

#[test]
fn private_fields_are_shown_all_or_none() {
    expect(&check(&changed("        -u32 secret\n", "")), &[]);
}

#[test]
fn variants_and_their_payloads_are_checked() {
    let added = changed("        Closed\n", "        Closed\n        Maybe\n");
    expect(
        &check(&added),
        &[(
            line_of(&added, "Maybe"),
            "shows the variant `Maybe`, which `State` does not have",
        )],
    );
    let dropped = changed("        Closed\n", "");
    expect(
        &check(&dropped),
        &[(
            line_of(&dropped, "class State"),
            "does not show the variant `Closed`",
        )],
    );
    let prose = changed("Fail(String)", "Fail(why)");
    expect(
        &check(&prose),
        &[(
            line_of(&prose, "Fail(why)"),
            "shows the variant `Fail(why)`; the code has `Fail(String)`",
        )],
    );
    let unit = changed("Fail(String)", "Fail");
    expect(
        &check(&unit),
        &[(
            line_of(&unit, "Fail"),
            "shows the variant `Fail`; the code has `Fail(String)`",
        )],
    );
    let renamed_field = changed(
        "Ended(how: String, code: i32)",
        "Ended(how: String, status: i32)",
    );
    expect(
        &check(&renamed_field),
        &[(
            line_of(&renamed_field, "Ended("),
            "the code has `Ended(how: String, code: i32)`",
        )],
    );
    expect(
        &check(&changed("Ended(how: String, code: i32)", "Ended(…)")),
        &[],
    );
    let unshown_unit = changed("        Pass\n", "        Pass(…)\n");
    expect(
        &check(&unshown_unit),
        &[(line_of(&unshown_unit, "Pass(…)"), "without parentheses")],
    );
}

#[test]
fn methods_are_checked_by_name_receiver_and_visibility() {
    let renamed = changed("+verdict(&self) Outcome", "+judge(&self) Outcome");
    expect(
        &check(&renamed),
        &[(line_of(&renamed, "judge"), "`Ticket` has no method `judge`")],
    );
    let receiver = changed("~close(&mut self)", "~close(&self)");
    expect(
        &check(&receiver),
        &[(
            line_of(&receiver, "close"),
            "shows `close` taking &self; the code's takes &mut self",
        )],
    );
    let visibility = changed("~close(&mut self)", "+close(&mut self)");
    expect(
        &check(&visibility),
        &[(
            line_of(&visibility, "close"),
            "shows `close` as `+`; the code makes it `~`",
        )],
    );
    let static_written = changed("+verdict(&self) Outcome", "+verdict()$ Outcome");
    expect(
        &check(&static_written),
        &[(
            line_of(&static_written, "verdict"),
            "taking no self; the code's takes &self",
        )],
    );
    expect(
        &check(&changed(
            "+new(title: String)$ Ticket",
            "+new(title) Ticket",
        )),
        &[],
    );
    let not_abstract = changed("+stop(&mut self)", "+stop(&mut self)*");
    expect(
        &check(&not_abstract),
        &[(
            line_of(&not_abstract, "stop"),
            "marks `stop` abstract (`*`); the code gives it a default body",
        )],
    );
    let abstract_lost = changed("+start(&mut self) bool*", "+start(&mut self) bool");
    expect(
        &check(&abstract_lost),
        &[(
            line_of(&abstract_lost, "class Run") + 2,
            "the code's has none",
        )],
    );
}

#[test]
fn kinds_and_names_resolve() {
    let kind = changed("        <<enumeration>>\n        Open\n", "        Open\n");
    let findings = check(&kind);
    assert!(
        findings
            .iter()
            .any(|f| f.message.contains("`State` is an enum")
                && f.message.contains("mark the class <<enumeration>>")),
        "{findings:#?}"
    );
    let ghost = changed("    class Form {", "    class Ghost\n    class Form {");
    expect(
        &check(&ghost),
        &[(
            line_of(&ghost, "class Ghost"),
            "class `Ghost` names no struct, enum or trait in crates/shapes",
        )],
    );
    let function = changed(
        "class qualify {\n        <<function>>\n        +qualify(",
        "class judge {\n        <<function>>\n        +judge(",
    )
    .replace("qualify ..> Ticket", "judge ..> Ticket");
    expect(
        &check(&function),
        &[(
            line_of(&function, "class judge"),
            "no function `judge` is in crates/shapes",
        )],
    );
}

#[test]
fn relations_are_held_by_fields_and_impls() {
    let multiplicity = changed("\"0..1\" Form", "\"*\" Form");
    expect(
        &check(&multiplicity),
        &[(
            line_of(&multiplicity, "\"*\" Form"),
            "the multiplicity \"*\" at `Form` does not fit `form: Option<Form>`",
        )],
    );
    let unheld = changed(
        "    Ticket --> State\n",
        "    Ticket --> State\n    Form --> State\n",
    );
    expect(
        &check(&unheld),
        &[(
            line_of(&unheld, "Form --> State"),
            "no field of `Form` names `State`",
        )],
    );
    let unimplemented = changed(
        "    Ticket ..|> Run\n",
        "    Ticket ..|> Run\n    Form ..|> Run\n",
    );
    expect(
        &check(&unimplemented),
        &[(
            line_of(&unimplemented, "Form ..|> Run"),
            "there is no `impl Run for Form`",
        )],
    );
}

#[test]
fn a_line_outside_the_subset_fails_at_its_line() {
    let unknown = changed(
        "    direction LR\n",
        "    direction LR\n    namespace Shapes {\n",
    );
    expect(
        &check(&unknown),
        &[(
            line_of(&unknown, "namespace"),
            "outside the class diagrams this check reads",
        )],
    );
}

#[test]
fn a_block_without_a_header_fails_at_its_fence() {
    expect(
        &check("flowchart TD\n    A --> B\n"),
        &[(3, "says not how it stays true")],
    );
    expect(
        &check("%% checked against crates/shapes\nflowchart TD\n"),
        &[(4, "`checked against` reads class and sequence diagrams")],
    );
    expect(
        &check("%% generated by tools/nothing.sh; do not edit\nflowchart TD\n"),
        &[(
            4,
            "`tools/nothing.sh`, which generates this block, does not exist",
        )],
    );
}

const SEQUENCE: &str = "%% checked against crates/shapes
sequenceDiagram
    actor You
    participant cli as kai (cli)
    participant store as shapes::store::Store
    participant module as shapes::store
    participant ticket as shapes::Ticket
    You->>cli: kai ticket list
    cli->>store: tickets()
    cli->>module: reset()
    cli->>ticket: verdict()
    ticket-->>cli: Outcome
    cli->>cli: prints
";

#[test]
fn a_sequence_names_what_it_calls() {
    expect(&check(SEQUENCE), &[]);
    let method = SEQUENCE.replace("tickets()", "rows()");
    expect(
        &check(&method),
        &[(
            line_of(&method, "rows()"),
            "calls `rows`; `shapes::store::Store` has no method `rows`",
        )],
    );
    let function = SEQUENCE.replace("reset()", "wipe()");
    expect(
        &check(&function),
        &[(
            line_of(&function, "wipe()"),
            "the module `shapes::store` has no function `wipe`",
        )],
    );
    let other_crate = SEQUENCE.replace("shapes::Ticket", "other::Ticket");
    expect(
        &check(&other_crate),
        &[(
            line_of(&other_crate, "other::Ticket"),
            "of the crate `other`, which `checked against` does not name",
        )],
    );
    let nothing = SEQUENCE.replace("shapes::Ticket", "shapes::Nothing");
    expect(
        &check(&nothing),
        &[(
            line_of(&nothing, "shapes::Nothing"),
            "names no module or type in crates/shapes",
        )],
    );
}

const STATES: &str = "%% shows every variant of shapes::Error but Task
stateDiagram-v2
    state \"BadName: nothing started\" as NotStarted
    [*] --> NotStarted
    NotStarted --> Exists : a file is there
";

#[test]
fn a_state_diagram_shows_every_variant() {
    expect(&check(STATES), &[]);
    let all = STATES.replace(" but Task", "");
    expect(
        &check(&all),
        &[(
            4,
            "no state shows `shapes::Error::Task` (crates/shapes/src/lib.rs:",
        )],
    );
    let gone = STATES.replace("but Task", "but Task, Gone");
    expect(
        &check(&gone),
        &[(4, "`shapes::Error` has no variant `Gone` to leave out")],
    );
}

/// A copy of the fixture whose `lib.rs` is `edit`ed, in a directory of its
/// own under the system's temporary directory.
fn copy_with(name: &str, edit: impl Fn(&str) -> String) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("diagrams-check-{}-{name}", std::process::id()));
    let src = dir.join("crates/shapes/src");
    std::fs::create_dir_all(&src).unwrap();
    let from = fixture().join("crates/shapes/src");
    let lib = std::fs::read_to_string(from.join("lib.rs")).unwrap();
    std::fs::write(src.join("lib.rs"), edit(&lib)).unwrap();
    std::fs::copy(from.join("store.rs"), src.join("store.rs")).unwrap();
    dir
}

#[test]
fn a_stamp_changes_with_the_code_only() {
    // The stamp the fixture has now, from the message that asks for one.
    let unstamped =
        "%% drawn from shapes::qualify, shapes::Ticket::verdict\nflowchart TD\n    A --> B\n";
    let findings = check(unstamped);
    assert_eq!(findings.len(), 2, "{findings:#?}");
    let stamps: Vec<String> = findings
        .iter()
        .map(|f| {
            assert!(f.message.contains("has no stamp"), "{f}");
            f.message
                .rsplit('#')
                .next()
                .unwrap()
                .trim_end_matches('`')
                .to_owned()
        })
        .collect();
    let stamped = format!(
        "%% drawn from shapes::qualify#{}, shapes::Ticket::verdict#{}\nflowchart TD\n    A --> B\n",
        stamps[0], stamps[1]
    );
    expect(&check(&stamped), &[]);

    let reformatted = copy_with("reformatted", |lib| {
        lib.replace(
            "/// Judges a ticket.",
            "/// Judges a ticket, again.\n// A comment.",
        )
        .replace(
            "    ticket.verdict()\n",
            "        ticket\n            .verdict()\n",
        )
    });
    expect(&check_at(&reformatted, &stamped), &[]);

    let changed_code = copy_with("changed", |lib| {
        lib.replace("    ticket.verdict()\n", "    Outcome::Pass\n")
    });
    let findings = check_at(&changed_code, &stamped);
    expect(
        &findings,
        &[(
            4,
            "the diagram is drawn from `shapes::qualify`, which changed. Look whether the diagram still holds",
        )],
    );
    assert!(
        findings[0].message.contains("set the stamp to #"),
        "{}",
        findings[0]
    );
    for dir in [reformatted, changed_code] {
        std::fs::remove_dir_all(dir).unwrap();
    }

    expect(
        &check("%% drawn from shapes::store\nflowchart TD\n"),
        &[(4, "`shapes::store` is a module")],
    );
    expect(
        &check("%% drawn from shapes::gone#01234567\nflowchart TD\n"),
        &[(
            4,
            "`shapes::gone`, which the diagram is drawn from, is not in crates/shapes",
        )],
    );
}
