//! docs/crates.md's graph is the one the manifests give, and the rules about
//! which crate may depend on which hold.
//!
//! `UPDATE_DIAGRAMS=1 cargo nextest run -p diagrams` rewrites the graph
//! after a change to a `[dependencies]` table; it never makes a broken rule
//! pass.

use std::path::Path;

use diagrams::graph::{self, PAGE};

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[test]
fn the_crate_graph_is_the_manifests() {
    let workspace = graph::read(root()).unwrap();
    assert!(
        workspace.crates.contains_key("types") && workspace.crates.contains_key("cli"),
        "the manifests read gave no crates: {workspace:?}"
    );
    let path = root().join(PAGE);
    let page = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{PAGE}: {err}"));
    let text = graph::flowchart(&workspace);
    if std::env::var_os("UPDATE_DIAGRAMS").is_some() {
        let new = graph::replace_block(&page, &text).unwrap();
        if new != page {
            std::fs::write(&path, new).unwrap();
        }
    } else {
        let shown = graph::generated_text(&page).unwrap();
        assert!(
            shown == text,
            "{PAGE} shows another crate graph than the manifests give; regenerate it with \
             UPDATE_DIAGRAMS=1 cargo nextest run -p diagrams\n--- shown\n{shown}--- the manifests\n{text}"
        );
    }
}

#[test]
fn the_crate_rules_hold() {
    let workspace = graph::read(root()).unwrap();
    let violations = graph::rule_violations(&workspace);
    assert!(
        violations.is_empty(),
        "the crate rules of {PAGE} do not hold:\n{}",
        violations.join("\n")
    );
}
