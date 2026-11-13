//! Every Mermaid block in the repository's Markdown says how it stays true
//! to the code, and does (`diagrams::check`). Reads source text only: no
//! crate of kai's is built, no network, no VM.

use std::path::Path;

use diagrams::BOTH_WAYS;
use diagrams::check::Checker;

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[test]
fn every_diagram_holds() {
    let checker = Checker::new(root());
    let files = checker.markdown_files();
    assert!(
        files
            .iter()
            .any(|file| file == Path::new("docs/qualify.md")),
        "the walk found no docs/qualify.md, so it would check nothing: {files:?}"
    );
    let findings: Vec<String> = files
        .iter()
        .flat_map(|file| checker.check_file(file))
        .map(|finding| finding.to_string())
        .collect();
    assert!(
        findings.is_empty(),
        "{} finding(s):\n{}\n{BOTH_WAYS}",
        findings.len(),
        findings.join("\n")
    );
}
