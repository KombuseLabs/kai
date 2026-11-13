//! Sequence diagrams: participants and messages.
//!
//! The subset: `participant` and `actor` lines (with `as`), messages with
//! the arrows `->>`, `-->>`, `->`, `-->`, `-x`, `--x`, `-)` and `--)` (an
//! activation `+` or `-` after the arrow is allowed), `Note`, `autonumber`,
//! `activate`, `deactivate`, and the blocks `loop`, `alt`/`else`, `opt`,
//! `par`/`and`, `critical`/`option`, `break`, `rect` and `box`, each closed
//! by `end`. Any other line is refused.
//!
//! ```
//! use diagrams::mermaid::sequence::parse;
//!
//! let diagram = parse("sequenceDiagram\n  participant store as store::Store\n  cli->>store: cursor(board)\n  store-->>cli: the cursor\n").unwrap();
//! assert_eq!(diagram.participant("store").unwrap().description, "store::Store");
//! assert_eq!(diagram.messages[0].label, "cursor(board)");
//! assert!(diagram.messages[1].reply);
//! ```

use super::{ParseError, is_decoration, significant};

/// A parsed sequence diagram.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SequenceDiagram {
    /// The participants, declared or named by a message first.
    pub participants: Vec<Participant>,
    /// The messages, in order.
    pub messages: Vec<Message>,
}

/// One participant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Participant {
    /// The name messages use.
    pub alias: String,
    /// What it shows: the text after `as`, or the alias.
    pub description: String,
    /// The line it is first named on.
    pub line: usize,
}

/// One message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// The sender's alias.
    pub from: String,
    /// The receiver's alias.
    pub to: String,
    /// Whether the arrow is dashed: a reply.
    pub reply: bool,
    /// The text after `:`.
    pub label: String,
    /// Its line.
    pub line: usize,
}

impl SequenceDiagram {
    /// The participant of that alias.
    pub fn participant(&self, alias: &str) -> Option<&Participant> {
        self.participants.iter().find(|p| p.alias == alias)
    }

    fn name(&mut self, alias: &str, line: usize) {
        if self.participant(alias).is_none() {
            self.participants.push(Participant {
                alias: alias.to_owned(),
                description: alias.to_owned(),
                line,
            });
        }
    }
}

/// The arrows, longest first so that `-->>` is not read as `-->`.
const ARROWS: [(&str, bool); 8] = [
    ("-->>", true),
    ("->>", false),
    ("--x", true),
    ("--)", true),
    ("-->", true),
    ("-x", false),
    ("-)", false),
    ("->", false),
];

/// Parses a sequence diagram.
pub fn parse(text: &str) -> Result<SequenceDiagram, ParseError> {
    let mut diagram = SequenceDiagram::default();
    let mut lines = significant(text);
    match lines.next() {
        Some((_, "sequenceDiagram")) => {}
        Some((line, first)) => {
            return Err(ParseError::new(
                line,
                format!("`{first}` where a sequence diagram starts with `sequenceDiagram`"),
            ));
        }
        None => return Err(ParseError::new(1, "the block holds no diagram")),
    }
    // The open blocks, by keyword.
    let mut blocks: Vec<&str> = Vec::new();
    for (line, text) in lines {
        let word = text.split_whitespace().next().unwrap_or("");
        match word {
            "participant" | "actor" => {
                let rest = text[word.len()..].trim();
                let (alias, description) = match rest.split_once(" as ") {
                    Some((alias, description)) => (alias.trim(), description.trim()),
                    None => (rest, rest),
                };
                if alias.is_empty() || alias.contains(char::is_whitespace) {
                    return Err(ParseError::new(
                        line,
                        format!("a {word} is written `{word} Alias as what it shows`"),
                    ));
                }
                if diagram.participant(alias).is_some() {
                    return Err(ParseError::new(
                        line,
                        format!(
                            "`{alias}` is declared after it was first named; declare participants first"
                        ),
                    ));
                }
                diagram.participants.push(Participant {
                    alias: alias.to_owned(),
                    description: description.to_owned(),
                    line,
                });
            }
            "Note" | "note" | "autonumber" | "activate" | "deactivate" => {}
            "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect" | "box" => {
                blocks.push(word);
            }
            "else" | "and" | "option" => {
                let opener = match word {
                    "else" => "alt",
                    "and" => "par",
                    _ => "critical",
                };
                if blocks.last() != Some(&opener) {
                    return Err(ParseError::new(
                        line,
                        format!("`{word}` outside an `{opener}` block"),
                    ));
                }
            }
            "end" => {
                if blocks.pop().is_none() {
                    return Err(ParseError::new(line, "`end` without a block"));
                }
            }
            _ if is_decoration(text) => {}
            _ => {
                let message = message(text, line)?;
                diagram.name(&message.from, line);
                diagram.name(&message.to, line);
                diagram.messages.push(message);
            }
        }
    }
    if let Some(open) = blocks.last() {
        return Err(ParseError::new(
            text.lines().count(),
            format!("a `{open}` block has no `end`"),
        ));
    }
    Ok(diagram)
}

/// A message line.
fn message(text: &str, line: usize) -> Result<Message, ParseError> {
    let refuse = || {
        ParseError::new(
            line,
            format!(
                "`{text}` is outside the sequence diagrams this check reads (participants, \
                 messages `A->>B: label`, notes, blocks); write it in that subset or extend the parser"
            ),
        )
    };
    let (head, label) = text.split_once(':').ok_or_else(refuse)?;
    let (at, arrow, reply) = ARROWS
        .iter()
        .filter_map(|(arrow, reply)| head.find(arrow).map(|at| (at, *arrow, *reply)))
        .min_by_key(|(at, arrow, _)| (*at, std::cmp::Reverse(arrow.len())))
        .ok_or_else(refuse)?;
    let from = head[..at].trim();
    let mut to = head[at + arrow.len()..].trim();
    if let Some(rest) = to.strip_prefix(['+', '-']) {
        to = rest.trim();
    }
    if from.is_empty()
        || to.is_empty()
        || from.contains(char::is_whitespace)
        || to.contains(char::is_whitespace)
        || from.contains(['<', '>'])
        || to.contains(['<', '>'])
    {
        return Err(refuse());
    }
    Ok(Message {
        from: from.to_owned(),
        to: to.to_owned(),
        reply,
        label: label.trim().to_owned(),
        line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_blocks_and_participants_parse() {
        let diagram = parse(
            "sequenceDiagram
    autonumber
    actor You
    participant cli as kai (cli)
    You->>cli: kai ticket qualify
    loop each ticket
        cli->>+engine: qualify(ticket)
        engine-->>-cli: Qualification
        Note over cli: prints
    end
    alt refused
        cli--)You: no
    else qualified
        cli-xYou: yes
    end
",
        )
        .unwrap();
        assert_eq!(diagram.participants.len(), 3);
        assert_eq!(diagram.participant("cli").unwrap().description, "kai (cli)");
        assert_eq!(diagram.messages.len(), 5);
        let call = &diagram.messages[1];
        assert_eq!(
            (call.from.as_str(), call.to.as_str(), call.reply),
            ("cli", "engine", false)
        );
        assert!(diagram.messages[2].reply);
        assert!(diagram.messages[3].reply);
        assert!(!diagram.messages[4].reply);
    }

    #[test]
    fn unknown_lines_and_open_blocks_are_refused() {
        for text in [
            "sequenceDiagram\ncreate participant A\n",
            "sequenceDiagram\nA<<->>B: both\n",
            "sequenceDiagram\nloop x\nA->>B: m\n",
            "sequenceDiagram\nend\n",
            "sequenceDiagram\nelse x\n",
            "sequenceDiagram\nA->>B\n",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }
}
