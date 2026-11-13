//! State diagrams: the states' names and descriptions.
//!
//! The subset: `stateDiagram` or `stateDiagram-v2`, `state "description"
//! as Name`, `state Name`, `state Name {` … `}`, `state Name <<choice>>`
//! (also `fork`, `join`), `Name : description`, transitions
//! `A --> B : label` with `[*]` at either end, `--` between concurrent
//! regions, `direction`, notes (one line, or several up to `end note`),
//! and styling. Any other line is refused.
//!
//! ```
//! use diagrams::mermaid::state::parse;
//!
//! let diagram = parse("stateDiagram-v2\n  state \"Open: the VM may start\" as Open\n  [*] --> Open\n  Open --> Closed : close()\n").unwrap();
//! assert_eq!(diagram.state("Open").unwrap().descriptions, ["Open: the VM may start"]);
//! assert!(diagram.state("Closed").is_some());
//! ```

use super::{ParseError, is_decoration, is_ident, significant, unquote};

/// A parsed state diagram.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateDiagram {
    /// The states, in the order they are first named.
    pub states: Vec<State>,
}

/// One state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    /// Its name.
    pub name: String,
    /// What it says about itself: `state "…" as Name` and `Name : …`.
    pub descriptions: Vec<String>,
    /// The line it is first named on.
    pub line: usize,
}

impl StateDiagram {
    /// The state of that name.
    pub fn state(&self, name: &str) -> Option<&State> {
        self.states.iter().find(|state| state.name == name)
    }

    fn named(&mut self, name: &str, line: usize) -> &mut State {
        let index = match self.states.iter().position(|state| state.name == name) {
            Some(index) => index,
            None => {
                self.states.push(State {
                    name: name.to_owned(),
                    descriptions: Vec::new(),
                    line,
                });
                self.states.len() - 1
            }
        };
        &mut self.states[index]
    }
}

/// Parses a state diagram.
pub fn parse(text: &str) -> Result<StateDiagram, ParseError> {
    let mut diagram = StateDiagram::default();
    let mut lines = significant(text);
    match lines.next() {
        Some((_, "stateDiagram" | "stateDiagram-v2")) => {}
        Some((line, first)) => {
            return Err(ParseError::new(
                line,
                format!("`{first}` where a state diagram starts with `stateDiagram-v2`"),
            ));
        }
        None => return Err(ParseError::new(1, "the block holds no diagram")),
    }
    let mut depth = 0usize;
    let mut in_note = false;
    for (line, text) in lines {
        if in_note {
            if text == "end note" {
                in_note = false;
            }
            continue;
        }
        let refuse = || {
            ParseError::new(
                line,
                format!(
                    "`{text}` is outside the state diagrams this check reads (states, \
                     descriptions, transitions, notes); write it in that subset or extend the parser"
                ),
            )
        };
        if is_decoration(text) || text == "--" || text.starts_with("direction ") {
            continue;
        }
        if let Some(rest) = text.strip_prefix("note ") {
            if !rest.contains(':') {
                in_note = true;
            }
            continue;
        }
        if text == "}" {
            depth = depth
                .checked_sub(1)
                .ok_or_else(|| ParseError::new(line, "`}` closes no state"))?;
            continue;
        }
        if text.starts_with("class ") {
            continue;
        }
        if let Some(rest) = text.strip_prefix("state ") {
            let rest = rest.trim();
            let (rest, opens) = match rest.strip_suffix('{') {
                Some(before) => (before.trim(), true),
                None => (rest, false),
            };
            let rest = match rest.split_once("<<") {
                Some((before, kind)) if matches!(kind.trim(), "choice>>" | "fork>>" | "join>>") => {
                    before.trim()
                }
                Some(_) => return Err(refuse()),
                None => rest,
            };
            if rest.starts_with('"') {
                let (description, name) = rest.rsplit_once(" as ").ok_or_else(refuse)?;
                let name = name.trim();
                if !is_ident(name) {
                    return Err(refuse());
                }
                diagram
                    .named(name, line)
                    .descriptions
                    .push(unquote(description).to_owned());
            } else if is_ident(rest) {
                diagram.named(rest, line);
            } else {
                return Err(refuse());
            }
            if opens {
                depth += 1;
            }
            continue;
        }
        if let Some((from, rest)) = text.split_once("-->") {
            let to = rest.split_once(':').map_or(rest, |(to, _label)| to).trim();
            let from = from.trim();
            for end in [from, to] {
                let end = end.split(":::").next().unwrap_or(end).trim();
                if end == "[*]" {
                    continue;
                }
                if !is_ident(end) {
                    return Err(refuse());
                }
                diagram.named(end, line);
            }
            continue;
        }
        if let Some((name, description)) = text.split_once(':') {
            let name = name.trim();
            if is_ident(name) {
                diagram
                    .named(name, line)
                    .descriptions
                    .push(description.trim().to_owned());
                continue;
            }
        }
        if is_ident(text) {
            diagram.named(text, line);
            continue;
        }
        return Err(refuse());
    }
    if depth > 0 || in_note {
        return Err(ParseError::new(
            text.lines().count(),
            "a composite state or a note is not closed",
        ));
    }
    Ok(diagram)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_and_their_descriptions_parse() {
        let diagram = parse(
            "stateDiagram-v2
    state \"BadName, Exists or Io: nothing started\" as NotStarted
    state Running {
        [*] --> Inner
    }
    [*] --> Checking
    Checking --> NotStarted : a check fails
    Checking : looks at the name
    note right of Checking
        several lines
    end note
    state pick <<choice>>
",
        )
        .unwrap();
        assert_eq!(
            diagram.state("NotStarted").unwrap().descriptions,
            ["BadName, Exists or Io: nothing started"]
        );
        assert_eq!(
            diagram.state("Checking").unwrap().descriptions,
            ["looks at the name"]
        );
        assert!(diagram.state("Inner").is_some());
        assert!(diagram.state("pick").is_some());
    }

    #[test]
    fn unknown_lines_are_refused() {
        for text in [
            "stateDiagram-v2\nA -> B\n",
            "stateDiagram-v2\nstate A <<other>>\n",
            "stateDiagram-v2\nstate A {\n",
            "stateDiagram-v2\n}\n",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }
}
