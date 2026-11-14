//! Rust stubs from planned class diagrams (`diagrams::stubs`, the program
//! `diagram-stubs`).
//!
//! - **The round trip**: the stubs of each diagram under
//!   `tests/fixtures/stubs/pass/`, and of the class diagram in
//!   `docs/qualify.md`, read back with syn, pass the class check against
//!   that diagram.
//! - **Refusals**: each diagram under `tests/fixtures/stubs/refuse/` is
//!   refused at the line its first line names (`%% refuse <line>: <text>`),
//!   with that text in the reason.
//! - **They build**: rustc type-checks the stubs of each fixture, and of
//!   `docs/qualify.md` with the one type it does not show added.
//!
//! No network, no VM; rustc is the toolchain's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use diagrams::markdown::mermaid_blocks;
use diagrams::mermaid::{Kind, class};
use diagrams::model::{Kind as ItemKind, TypeDef, compare, diagram_classes, relation_problems};
use diagrams::rust::CrateModel;
use diagrams::stubs::{Stubs, generate};

fn crate_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures(kind: &str) -> Vec<PathBuf> {
    let dir = crate_dir().join("tests/fixtures/stubs").join(kind);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("{}: {err}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "mmd"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no fixtures in {}", dir.display());
    files
}

fn stubs_of(text: &str, label: &str) -> (class::ClassDiagram, Stubs) {
    let diagram = class::parse(text).unwrap_or_else(|err| panic!("{label}: {err}"));
    let stubs =
        generate(&diagram).unwrap_or_else(|problems| panic!("{label}: refused: {problems:?}"));
    (diagram, stubs)
}

/// What the class check says of `diagram` against `code`, as the check in
/// `docs/` would say it.
fn check_against(diagram: &class::ClassDiagram, code: &str, label: &str) -> Vec<String> {
    let model = CrateModel::from_source("stubs", "stubs.rs", code)
        .unwrap_or_else(|err| panic!("{label}: the stubs do not parse: {err}"));
    let (classes, problems) = diagram_classes(diagram);
    assert!(problems.is_empty(), "{label}: {problems:?}");
    let mut found: BTreeMap<String, TypeDef> = BTreeMap::new();
    let mut out = Vec::new();
    for class in classes.iter().filter(|class| class.code) {
        let name = &class.def.name;
        let def = if class.def.kind == ItemKind::Function {
            match model.functions_named(name).as_slice() {
                [function] => model.function_def(function),
                other => {
                    out.push(format!("{name}: {} functions of that name", other.len()));
                    continue;
                }
            }
        } else {
            match model.types_named(name).as_slice() {
                [ty] => ty.def.clone(),
                other => {
                    out.push(format!("{name}: {} types of that name", other.len()));
                    continue;
                }
            }
        };
        out.extend(
            compare(class, &def)
                .into_iter()
                .map(|p| format!("line {}: {}", p.line, p.message)),
        );
        found.insert(name.clone(), def);
    }
    let lookup = |name: &str| found.get(name);
    out.extend(
        relation_problems(diagram, &classes, &lookup)
            .into_iter()
            .map(|p| format!("line {}: {}", p.line, p.message)),
    );
    out
}

/// rustc's verdict on `code` as a library: type-checked, with unused
/// parameters and unfulfilled `expect`s as errors.
fn rustc(code: &str, name: &str) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("diagram-stubs-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("lib.rs");
    std::fs::write(&source, code).unwrap();
    let output = Command::new("rustc")
        .current_dir(crate_dir())
        .args([
            "--edition",
            "2024",
            "--crate-type",
            "lib",
            "--crate-name",
            "stubs",
            "--emit",
            "metadata",
        ])
        .args([
            "-D",
            "unused_variables",
            "-D",
            "unfulfilled_lint_expectations",
        ])
        .arg("-o")
        .arg(dir.join("stubs.rmeta"))
        .arg(&source)
        .output()
        .expect("running rustc");
    std::fs::remove_dir_all(&dir).ok();
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn qualify_diagram() -> String {
    let path = crate_dir().join("../../docs/qualify.md");
    let text = std::fs::read_to_string(&path).unwrap();
    let blocks: Vec<_> = mermaid_blocks(&text)
        .into_iter()
        .filter(|block| Kind::of(&block.text).0 == Kind::Class)
        .collect();
    assert_eq!(blocks.len(), 1, "docs/qualify.md has one class diagram");
    blocks[0].text.clone()
}

#[test]
fn the_stubs_of_each_fixture_pass_the_check_against_it() {
    for path in fixtures("pass") {
        let label = path.display().to_string();
        let (diagram, stubs) = stubs_of(&std::fs::read_to_string(&path).unwrap(), &label);
        let found = check_against(&diagram, &stubs.code, &label);
        assert!(
            found.is_empty(),
            "{label}:\n{}\n{}",
            found.join("\n"),
            stubs.code
        );
        assert!(
            stubs.not_shown.is_empty(),
            "{label} is self-contained: {:?}",
            stubs.not_shown
        );
    }
}

#[test]
fn the_stubs_of_each_fixture_build() {
    for path in fixtures("pass") {
        let label = path.display().to_string();
        let (_, stubs) = stubs_of(&std::fs::read_to_string(&path).unwrap(), &label);
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        if let Err(errors) = rustc(&stubs.code, &name) {
            panic!(
                "{label}: rustc refused the stubs:\n{errors}\n{}",
                stubs.code
            );
        }
    }
}

#[test]
fn the_types_of_docs_qualify_make_the_round_trip() {
    let (diagram, stubs) = stubs_of(&qualify_diagram(), "docs/qualify.md");
    let found = check_against(&diagram, &stubs.code, "docs/qualify.md");
    assert!(found.is_empty(), "{}", found.join("\n"));
    assert_eq!(stubs.not_shown, ["RepoName"]);
    let model = CrateModel::from_source("stubs", "stubs.rs", &stubs.code).unwrap();
    let mut names: Vec<&str> = model.types.iter().map(|ty| ty.def.name.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "Access",
            "ActorAccess",
            "BoardFiles",
            "Facts",
            "Finding",
            "Form",
            "Issue",
            "IssueId",
            "IssueState",
            "LabelEvent",
            "Outcome",
            "Qualification",
            "Rule",
            "Ticket",
            "Verdict"
        ]
    );
    assert_eq!(model.functions_named("qualify").len(), 1);
}

#[test]
fn the_types_of_docs_qualify_build_once_the_type_it_does_not_show_exists() {
    let (_, stubs) = stubs_of(&qualify_diagram(), "docs/qualify.md");
    let code = format!("{}\npub struct RepoName;\n", stubs.code);
    if let Err(errors) = rustc(&code, "qualify") {
        panic!("rustc refused the stubs of docs/qualify.md:\n{errors}\n{code}");
    }
}

#[test]
fn each_refusal_is_at_its_line() {
    for path in fixtures("refuse") {
        let label = path.display().to_string();
        let text = std::fs::read_to_string(&path).unwrap();
        let expected = text
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("%% refuse "))
            .unwrap_or_else(|| panic!("{label}: the first line is `%% refuse <line>: <text>`"));
        let (line, why) = expected.split_once(": ").unwrap();
        let line: usize = line.parse().unwrap();
        let reasons: Vec<(usize, String)> = match class::parse(&text) {
            Err(err) => vec![(err.line, err.message)],
            Ok(diagram) => match generate(&diagram) {
                Ok(stubs) => panic!("{label} was not refused:\n{}", stubs.code),
                Err(problems) => problems.into_iter().map(|p| (p.line, p.message)).collect(),
            },
        };
        assert!(
            reasons
                .iter()
                .any(|(at, message)| *at == line && message.contains(why)),
            "{label}: expected line {line} with `{why}`, got {reasons:?}"
        );
    }
}

#[test]
fn the_program_prints_stubs_or_nothing() {
    let program = env!("CARGO_BIN_EXE_diagram-stubs");
    let pass = crate_dir().join("tests/fixtures/stubs/pass/traits.mmd");
    let output = Command::new(program).arg(&pass).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let printed = String::from_utf8(output.stdout).unwrap();
    syn::parse_file(&printed).expect("the printed stubs parse");
    assert!(
        printed.contains("pub trait Runner {\n"),
        "rustfmt formatted it:\n{printed}"
    );

    let refused = crate_dir().join("tests/fixtures/stubs/refuse/multiplicity.mmd");
    let output = Command::new(program).arg(&refused).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "nothing on standard output when refused"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("multiplicity.mmd:7: "), "{stderr}");

    let qualify = crate_dir().join("../../docs/qualify.md");
    let output = Command::new(program).arg(&qualify).output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not shown (define or import them): RepoName"),
        "{stderr}"
    );
}

/// The fixture `pass/defaults.mmd`'s stubs: what decides `Self: Sized` and
/// the `expect` of unused parameters, which the build test then holds to
/// rustc (`-D unfulfilled_lint_expectations`).
#[test]
fn defaults_take_self_sized_and_unused_parameters_are_expected_only_when_rustc_warns() {
    let path = crate_dir().join("tests/fixtures/stubs/pass/defaults.mmd");
    let (_, stubs) = stubs_of(&std::fs::read_to_string(&path).unwrap(), "defaults.mmd");
    let code = &stubs.code;
    // The line after `name`'s signature, and whether an `expect` precedes it.
    let at = |name: &str| {
        let start = code
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("no fn {name} in:\n{code}"));
        let line_start = code[..start].rfind('\n').map_or(0, |i| i + 1);
        let before = code[..line_start].trim_end().lines().last().unwrap_or("");
        let signature = &code[start..start + code[start..].find('{').unwrap()];
        (
            before.contains("expect(unused_variables"),
            signature.to_owned(),
        )
    };
    for sized in ["merge", "merge_all", "pick", "pair"] {
        let (_, signature) = at(sized);
        assert!(
            signature.contains("where Self: Sized"),
            "{sized}: {signature}"
        );
    }
    let (_, compare) = at("compare");
    assert!(
        !compare.contains("Sized"),
        "a `&Self` needs no `Sized`: {compare}"
    );
    for (name, expected) in [
        ("work", false),
        ("mix", true),
        ("add", true),
        ("tick", false),
        ("tock", true),
    ] {
        let (expect, signature) = at(name);
        assert_eq!(expect, expected, "{name}: {signature}");
    }
    rustc(code, "defaults").unwrap_or_else(|err| panic!("defaults.mmd's stubs:\n{err}\n{code}"));
}
