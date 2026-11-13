//! Class diagrams: classes, their members as written, relations and notes.
//!
//! The subset:
//!
//! - `classDiagram`, `direction LR`;
//! - `class Name`, `class Name~T~`, `class Name {` … `}`, with
//!   `<<stereotype>>` lines and member lines inside; `class Name:::css`;
//! - `<<stereotype>> Name` and `Name : member` outside a class body;
//! - relations `A "1" *-- "0..1" B : label` with `*--`, `o--`, `-->`, `--`,
//!   `..>`, `..|>` and `..`, each also written the other way round
//!   (`--*`, `<--`, `<|..`, …), spaces around the arrow;
//! - `note for Name "text"` and `note "text"`;
//! - styling and accessibility lines, which are dropped.
//!
//! Inheritance (`<|--`) has no form in Rust and is refused, as is every
//! other line. What a member line means (a field, a method, a variant)
//! depends on its class's stereotype, and is [`crate::model`]'s to say.
//!
//! ```
//! use diagrams::mermaid::class::{parse, RelationKind};
//!
//! let diagram = parse("classDiagram\n  class Ticket {\n    +String title\n  }\n  Ticket \"1\" *-- \"0..1\" Form : the body\n").unwrap();
//! assert_eq!(diagram.classes[0].members[0].text, "+String title");
//! let relation = &diagram.relations[0];
//! assert_eq!((relation.from.as_str(), relation.to.as_str()), ("Ticket", "Form"));
//! assert_eq!(relation.kind, RelationKind::Composition);
//! assert_eq!(relation.to_multiplicity.as_deref(), Some("0..1"));
//! ```

use super::{ParseError, is_decoration, is_ident, significant, unquote};

/// A parsed class diagram.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassDiagram {
    /// The classes, in the order they first appear; a class a relation
    /// names without declaring it is here too.
    pub classes: Vec<Class>,
    /// The relations, in order.
    pub relations: Vec<Relation>,
    /// The notes, in order.
    pub notes: Vec<Note>,
}

/// One class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Class {
    /// Its name.
    pub name: String,
    /// Its generic parameters, from `Name~T, U~`.
    pub generics: Vec<String>,
    /// Its stereotypes, without the angle brackets: `enumeration`.
    pub stereotypes: Vec<String>,
    /// Its member lines, as written.
    pub members: Vec<Member>,
    /// The line it is first named on.
    pub line: usize,
    /// Whether a `class` line declares it (else only a relation names it).
    pub declared: bool,
}

/// A member line of a class, as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The text, trimmed.
    pub text: String,
    /// Its line.
    pub line: usize,
}

/// The kinds of relation in the subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationKind {
    /// `*--`: the whole (`from`) holds the part (`to`).
    Composition,
    /// `o--`: the whole (`from`) holds the part (`to`).
    Aggregation,
    /// `-->`: `from` knows `to`.
    Association,
    /// `--`: a solid line without meaning in Rust.
    Link,
    /// `..>`: `from` uses or returns `to`.
    Dependency,
    /// `..|>`: `from` implements `to`.
    Realization,
    /// `..`: a dashed line without meaning in Rust.
    DashedLink,
}

/// One relation, with `from` the side the arrow's meaning starts from
/// whichever way it is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    /// The whole, the user or the implementor.
    pub from: String,
    /// The part, the used or the implemented.
    pub to: String,
    /// What kind.
    pub kind: RelationKind,
    /// The multiplicity written at `from`.
    pub from_multiplicity: Option<String>,
    /// The multiplicity written at `to`.
    pub to_multiplicity: Option<String>,
    /// The label after `:`.
    pub label: Option<String>,
    /// Its line.
    pub line: usize,
}

/// A note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// The class it is for, if any.
    pub class: Option<String>,
    /// Its text, unquoted.
    pub text: String,
    /// Its line.
    pub line: usize,
}

impl ClassDiagram {
    /// The class of that name.
    pub fn class(&self, name: &str) -> Option<&Class> {
        self.classes.iter().find(|class| class.name == name)
    }

    /// The note for the class of that name, if it has one.
    pub fn note_for(&self, name: &str) -> Option<&Note> {
        self.notes
            .iter()
            .find(|note| note.class.as_deref() == Some(name))
    }

    fn class_index(&mut self, name: &str, line: usize) -> usize {
        match self.classes.iter().position(|class| class.name == name) {
            Some(index) => index,
            None => {
                self.classes.push(Class {
                    name: name.to_owned(),
                    generics: Vec::new(),
                    stereotypes: Vec::new(),
                    members: Vec::new(),
                    line,
                    declared: false,
                });
                self.classes.len() - 1
            }
        }
    }
}

/// Parses a class diagram.
pub fn parse(text: &str) -> Result<ClassDiagram, ParseError> {
    let mut diagram = ClassDiagram::default();
    let mut lines = significant(text);
    match lines.next() {
        Some((_, "classDiagram" | "classDiagram-v2")) => {}
        Some((line, first)) => {
            return Err(ParseError::new(
                line,
                format!("`{first}` where a class diagram starts with `classDiagram`"),
            ));
        }
        None => return Err(ParseError::new(1, "the block holds no diagram")),
    }
    // The class whose body is open, and whether it had a body before.
    let mut body: Option<usize> = None;
    let mut bodies: Vec<String> = Vec::new();
    for (line, text) in lines {
        if let Some(index) = body {
            if text == "}" {
                body = None;
            } else if let Some(stereotype) = stereotype(text) {
                diagram.classes[index]
                    .stereotypes
                    .push(stereotype.to_owned());
            } else if text.contains('{') || text.contains('}') {
                return Err(ParseError::new(
                    line,
                    "a member line holds no `{` or `}`; a class body ends with `}` on a line of its own",
                ));
            } else {
                diagram.classes[index].members.push(Member {
                    text: text.to_owned(),
                    line,
                });
            }
            continue;
        }
        if is_decoration(text) {
            continue;
        }
        if let Some(rest) = text.strip_prefix("direction ") {
            if !matches!(rest.trim(), "TB" | "TD" | "BT" | "LR" | "RL") {
                return Err(ParseError::new(line, format!("`{rest}` is no direction")));
            }
        } else if let Some(rest) = text.strip_prefix("class ") {
            let (name, generics, opens) = declaration(rest, line)?;
            if opens {
                if bodies.contains(&name) {
                    return Err(ParseError::new(
                        line,
                        format!("class `{name}` has a body twice; one class, one body"),
                    ));
                }
                bodies.push(name.clone());
            }
            let index = diagram.class_index(&name, line);
            let class = &mut diagram.classes[index];
            class.declared = true;
            if !generics.is_empty() {
                if !class.generics.is_empty() && class.generics != generics {
                    return Err(ParseError::new(
                        line,
                        format!("class `{name}` is declared with two sets of generics"),
                    ));
                }
                class.generics = generics;
            }
            if opens {
                body = Some(index);
            }
        } else if let Some(rest) = text.strip_prefix("note ") {
            diagram.notes.push(note(rest, line)?);
        } else if let Some((stereotype, rest)) = leading_stereotype(text) {
            let name = rest.trim();
            if !is_ident(name) {
                return Err(ParseError::new(
                    line,
                    format!("`<<{stereotype}>>` is followed by a class name, not `{name}`"),
                ));
            }
            let index = diagram.class_index(name, line);
            diagram.classes[index]
                .stereotypes
                .push(stereotype.to_owned());
        } else if let Some(relation) = relation(text, line) {
            let relation = relation?;
            diagram.class_index(&relation.from, line);
            diagram.class_index(&relation.to, line);
            diagram.relations.push(relation);
        } else if let Some((name, member)) = text.split_once(':')
            && is_ident(name.trim())
            && !member.trim().is_empty()
        {
            let index = diagram.class_index(name.trim(), line);
            let member = member.trim();
            if let Some(stereotype) = stereotype(member) {
                diagram.classes[index]
                    .stereotypes
                    .push(stereotype.to_owned());
            } else {
                diagram.classes[index].members.push(Member {
                    text: member.to_owned(),
                    line,
                });
            }
        } else {
            return Err(ParseError::new(
                line,
                format!(
                    "`{text}` is outside the class diagrams this check reads (classes, members, \
                     stereotypes, relations, notes); write it in that subset or extend the parser"
                ),
            ));
        }
    }
    if body.is_some() {
        return Err(ParseError::new(
            text.lines().count(),
            "a class body has no closing `}`",
        ));
    }
    Ok(diagram)
}

/// `<<name>>` alone on a line.
fn stereotype(text: &str) -> Option<&str> {
    let inner = text.strip_prefix("<<")?.strip_suffix(">>")?.trim();
    (!inner.is_empty() && !inner.contains(['<', '>'])).then_some(inner)
}

/// `<<name>> rest`.
fn leading_stereotype(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("<<")?;
    let (inner, after) = rest.split_once(">>")?;
    let inner = inner.trim();
    (!inner.is_empty() && !after.trim().is_empty()).then_some((inner, after))
}

/// What follows `class `: the name, its generics, and whether a body opens.
fn declaration(rest: &str, line: usize) -> Result<(String, Vec<String>, bool), ParseError> {
    let mut rest = rest.trim();
    let mut opens = false;
    if let Some(before) = rest.strip_suffix('{') {
        opens = true;
        rest = before.trim();
    }
    if let Some((before, _css)) = rest.split_once(":::") {
        rest = before.trim();
    }
    if rest.contains('[') || rest.contains('"') {
        return Err(ParseError::new(
            line,
            "a class label (`class Name[\"…\"]`) is outside the subset; the class's name is what the check reads",
        ));
    }
    let (name, generics) = match rest.split_once('~') {
        Some((name, generics)) => {
            let generics = generics
                .strip_suffix('~')
                .ok_or_else(|| ParseError::new(line, "a class's generics are written `Name~T~`"))?;
            let generics: Vec<String> = generics
                .split(',')
                .map(|generic| generic.trim().to_owned())
                .collect();
            if generics.iter().any(|generic| !is_ident(generic)) {
                return Err(ParseError::new(
                    line,
                    "a class's generics are names, as in `Name~T, U~`",
                ));
            }
            (name.trim(), generics)
        }
        None => (rest, Vec::new()),
    };
    if !is_ident(name) {
        return Err(ParseError::new(line, format!("`{name}` is no class name")));
    }
    Ok((name.to_owned(), generics, opens))
}

/// What follows `note `.
fn note(rest: &str, line: usize) -> Result<Note, ParseError> {
    let rest = rest.trim();
    if let Some(after) = rest.strip_prefix("for ") {
        let after = after.trim();
        let (name, text) = after
            .split_once(char::is_whitespace)
            .ok_or_else(|| ParseError::new(line, "a note is written `note for Name \"text\"`"))?;
        if !is_ident(name) {
            return Err(ParseError::new(line, format!("`{name}` is no class name")));
        }
        Ok(Note {
            class: Some(name.to_owned()),
            text: unquote(text).to_owned(),
            line,
        })
    } else if rest.starts_with('"') {
        Ok(Note {
            class: None,
            text: unquote(rest).to_owned(),
            line,
        })
    } else {
        Err(ParseError::new(
            line,
            "a note is written `note for Name \"text\"` or `note \"text\"`",
        ))
    }
}

/// The arrows of the subset: the kind, and whether the arrow's meaning
/// starts on its right (written the other way round).
fn arrow(token: &str) -> Option<Result<(RelationKind, bool), String>> {
    let is_arrow = (token.contains("--") || token.contains(".."))
        && token.chars().all(|c| "<>|*o.-".contains(c));
    if !is_arrow {
        return None;
    }
    use RelationKind::*;
    Some(match token {
        "*--" => Ok((Composition, false)),
        "--*" => Ok((Composition, true)),
        "o--" => Ok((Aggregation, false)),
        "--o" => Ok((Aggregation, true)),
        "-->" => Ok((Association, false)),
        "<--" => Ok((Association, true)),
        "--" => Ok((Link, false)),
        "..>" => Ok((Dependency, false)),
        "<.." => Ok((Dependency, true)),
        "..|>" => Ok((Realization, false)),
        "<|.." => Ok((Realization, true)),
        ".." => Ok((DashedLink, false)),
        "<|--" | "--|>" => Err(
            "inheritance has no form in Rust; a type that implements a trait is drawn `Type ..|> Trait`"
                .to_owned(),
        ),
        other => Err(format!(
            "`{other}` is outside the relations this check reads (*--, o--, -->, --, ..>, ..|>, .., and the same the other way round)"
        )),
    })
}

/// A relation line, if `text` is one.
fn relation(text: &str, line: usize) -> Option<Result<Relation, ParseError>> {
    let (left, label) = split_label(text);
    let tokens = tokens(left);
    let position = tokens.iter().position(|token| arrow(token).is_some());
    let Some(position) = position else {
        if left.contains("--") || left.contains("..") {
            return Some(Err(ParseError::new(
                line,
                "a relation is written with spaces around its arrow, as in `A --> B`",
            )));
        }
        return None;
    };
    Some(build_relation(&tokens, position, label, line))
}

fn build_relation(
    tokens: &[String],
    position: usize,
    label: Option<&str>,
    line: usize,
) -> Result<Relation, ParseError> {
    let (kind, reversed) = arrow(&tokens[position])
        .expect("the position is an arrow's")
        .map_err(|message| ParseError::new(line, message))?;
    let shape = || {
        ParseError::new(
            line,
            "a relation is written `A \"1\" *-- \"0..1\" B : label`, the multiplicities quoted and optional",
        )
    };
    let (left, right) = (&tokens[..position], &tokens[position + 1..]);
    let (left_name, left_multiplicity) = match left {
        [name] => (name, None),
        [name, multiplicity] if multiplicity.starts_with('"') => {
            (name, Some(unquote(multiplicity)))
        }
        _ => return Err(shape()),
    };
    let (right_name, right_multiplicity) = match right {
        [name] => (name, None),
        [multiplicity, name] if multiplicity.starts_with('"') => {
            (name, Some(unquote(multiplicity)))
        }
        _ => return Err(shape()),
    };
    for name in [left_name, right_name] {
        if !is_ident(name) {
            return Err(ParseError::new(line, format!("`{name}` is no class name")));
        }
    }
    let (from, to, from_multiplicity, to_multiplicity) = if reversed {
        (right_name, left_name, right_multiplicity, left_multiplicity)
    } else {
        (left_name, right_name, left_multiplicity, right_multiplicity)
    };
    Ok(Relation {
        from: from.clone(),
        to: to.clone(),
        kind,
        from_multiplicity: from_multiplicity.map(str::to_owned),
        to_multiplicity: to_multiplicity.map(str::to_owned),
        label: label.map(str::to_owned),
        line,
    })
}

/// The text before the first `:` outside quotes, and the label after it.
fn split_label(text: &str) -> (&str, Option<&str>) {
    let mut quoted = false;
    for (index, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => {
                let label = text[index + 1..].trim();
                return (&text[..index], (!label.is_empty()).then_some(label));
            }
            _ => {}
        }
    }
    (text, None)
}

/// Words split at whitespace outside quotes; a quoted word keeps its quotes.
fn tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        if c == '"' {
            quoted = !quoted;
            current.push(c);
        } else if c.is_whitespace() && !quoted {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIAGRAM: &str = "classDiagram
    direction LR
    %% a comment
    class Store~T~ {
        <<trait>>
        +get(&self) T
    }
    class Error {
        <<enumeration>>
        Io(String)
    }
    <<excerpt>> Policy
    Policy : +Vec~String~ net
    Policy --> Error
    Error <|.. Policy
    Egress --* Policy : holds
    note for Egress \"the run's way out\"
    note \"a free note\"
    classDef built fill:#d4edda
";

    #[test]
    fn the_subset_parses() {
        let diagram = parse(DIAGRAM).unwrap();
        let store = diagram.class("Store").unwrap();
        assert_eq!(store.generics, ["T"]);
        assert_eq!(store.stereotypes, ["trait"]);
        assert_eq!(store.members[0].text, "+get(&self) T");
        assert_eq!(store.members[0].line, 6);
        let policy = diagram.class("Policy").unwrap();
        assert_eq!(policy.stereotypes, ["excerpt"]);
        assert_eq!(policy.members[0].text, "+Vec~String~ net");
        assert!(!policy.declared);
        let realization = &diagram.relations[1];
        assert_eq!(realization.kind, RelationKind::Realization);
        assert_eq!(
            (realization.from.as_str(), realization.to.as_str()),
            ("Policy", "Error")
        );
        let composition = &diagram.relations[2];
        assert_eq!(
            (composition.from.as_str(), composition.to.as_str()),
            ("Policy", "Egress")
        );
        assert_eq!(composition.label.as_deref(), Some("holds"));
        assert_eq!(
            diagram.note_for("Egress").unwrap().text,
            "the run's way out"
        );
        assert_eq!(diagram.notes[1].class, None);
    }

    #[test]
    fn unknown_lines_are_refused() {
        for (line, why) in [
            ("namespace Shapes {", "outside the class diagrams"),
            ("A <|-- B", "inheritance"),
            ("A <--> B", "outside the relations"),
            ("A-->B", "spaces around"),
            ("class A[\"label\"]", "class label"),
            ("click A href \"x\"", "outside the class diagrams"),
            ("A \"1\" \"2\" *-- B", "multiplicities"),
        ] {
            let err = parse(&format!("classDiagram\n{line}\n")).unwrap_err();
            assert_eq!(err.line, 2, "{line}");
            assert!(err.message.contains(why), "{line}: {}", err.message);
        }
    }

    #[test]
    fn a_body_twice_and_an_open_body_are_refused() {
        let twice = "classDiagram\nclass A {\n+u8 x\n}\nclass A {\n}\n";
        assert!(parse(twice).unwrap_err().message.contains("body twice"));
        assert!(
            parse("classDiagram\nclass A {\n+u8 x\n")
                .unwrap_err()
                .message
                .contains("closing")
        );
    }

    #[test]
    fn a_diagram_of_another_kind_is_refused() {
        assert!(parse("flowchart TD\n").is_err());
    }
}
