//! The subset of Mermaid kai's diagrams use.
//!
//! Each parser reads one kind of diagram and refuses any line outside its
//! subset, with the line's number: a construct it does not know could say
//! something the checks would then miss. Styling (`classDef`, `style`,
//! `cssClass`), comments and accessibility lines are read and dropped.
//! Line numbers are within the block, from 1.

pub mod class;
pub mod sequence;
pub mod state;

use std::fmt;

/// What kind of diagram a block is, from its first line that is not a
/// comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `classDiagram`.
    Class,
    /// `sequenceDiagram`.
    Sequence,
    /// `stateDiagram` or `stateDiagram-v2`.
    State,
    /// `flowchart` or `graph`.
    Flowchart,
    /// Any other, by its keyword.
    Other(String),
    /// A block with no diagram in it.
    Empty,
}

impl Kind {
    /// The kind of the diagram in `text`, and the line its keyword is on.
    pub fn of(text: &str) -> (Kind, usize) {
        let Some((index, line)) = significant(text).next() else {
            return (Kind::Empty, 0);
        };
        let word = line.split_whitespace().next().unwrap_or("");
        let kind = match word {
            "classDiagram" | "classDiagram-v2" => Kind::Class,
            "sequenceDiagram" => Kind::Sequence,
            "stateDiagram" | "stateDiagram-v2" => Kind::State,
            "flowchart" | "graph" => Kind::Flowchart,
            other => Kind::Other(other.to_owned()),
        };
        (kind, index)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Kind::Class => f.write_str("class diagram"),
            Kind::Sequence => f.write_str("sequence diagram"),
            Kind::State => f.write_str("state diagram"),
            Kind::Flowchart => f.write_str("flowchart"),
            Kind::Other(word) => write!(f, "`{word}` diagram"),
            Kind::Empty => f.write_str("empty block"),
        }
    }
}

/// A line a parser does not take, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// The line within the block, from 1.
    pub line: usize,
    /// What is wrong with it.
    pub message: String,
}

impl ParseError {
    pub(crate) fn new(line: usize, message: impl Into<String>) -> ParseError {
        ParseError {
            line,
            message: message.into(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// The lines that say something: numbered from 1, trimmed, without blank
/// lines, `%%` comments and `%%{…}%%` directives.
pub(crate) fn significant(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with("%%"))
}

/// Whether `word` is an identifier as kai's diagrams use them.
pub(crate) fn is_ident(word: &str) -> bool {
    let mut chars = word.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// The lines every parser reads and drops: styling and accessibility.
pub(crate) fn is_decoration(line: &str) -> bool {
    let word = line.split_whitespace().next().unwrap_or("");
    matches!(word, "classDef" | "cssClass" | "style" | "linkStyle")
        || line.starts_with("accTitle")
        || line.starts_with("accDescr")
}

/// Unquotes `"text"`; other text is returned trimmed.
pub(crate) fn unquote(text: &str) -> &str {
    let text = text.trim();
    text.strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_is_the_first_keyword() {
        assert_eq!(
            Kind::of("%% checked against crates/a\n\nclassDiagram\n"),
            (Kind::Class, 3)
        );
        assert_eq!(Kind::of("graph LR\n").0, Kind::Flowchart);
        assert_eq!(Kind::of("pie\n").0, Kind::Other("pie".into()));
        assert_eq!(Kind::of("%% only a comment\n").0, Kind::Empty);
    }
}
