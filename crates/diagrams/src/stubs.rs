//! Rust stubs from a planned class diagram: the class check read
//! backwards, so that the stubs pass it by construction.
//!
//! [`generate`] checks the diagram against itself first, with the same
//! rules the check keeps ([`crate::model::diagram_classes`] and
//! [`crate::model::relation_problems`], the diagram's own classes standing
//! in for the code), and refuses what it cannot turn into Rust that
//! builds. Then it writes one stretch of Rust in the diagram's order:
//!
//! | In the diagram | In the stubs |
//! | --- | --- |
//! | a class without a stereotype | a `pub struct` with every field shown |
//! | `<<enumeration>>` | a `pub enum`: `Name`, `Name(T, U)`, `Name { a: T }` |
//! | `<<trait>>` | a `pub trait`; a method marked `*` has no body, any other a `todo!()` default |
//! | `<<function>>` | each method a free function |
//! | `A ..\|> B`, B a `<<trait>>` | `impl B for A`, a `todo!()` for each of B's methods without a default |
//! | `note for A "…"` | A's doc comment |
//! | a method's `$`, `self`, `&self`, `&mut self` | that receiver; with none written, `&self` |
//! | another stereotype | nothing: not code |
//!
//! Every body is `todo!()`; a function with parameters gets
//! `#[expect(unused_variables, reason = "a stub from a diagram")]`, which
//! rustc reports once the body uses them. No `mod`, no `use`, no file per
//! type: where the items go is the author's call. Derives, doc comments
//! beyond a note, and the imports of types the diagram does not show are
//! left to the author; [`Stubs::not_shown`] lists those types.
//!
//! ```
//! use diagrams::mermaid::class::parse;
//!
//! let diagram = parse("classDiagram\nclass Ticket {\n  +String title\n  +close(&mut self)\n}\n").unwrap();
//! let stubs = diagrams::stubs::generate(&diagram).unwrap();
//! assert!(stubs.code.contains("pub struct Ticket {\n    pub title: String,\n}"));
//! assert!(stubs.code.contains("pub fn close(&mut self) {\n        todo!()\n    }"));
//! assert!(stubs.not_shown.is_empty());
//! ```

use std::collections::BTreeSet;

use crate::mermaid::class::{ClassDiagram, Member};
use crate::model::{
    self, DiagramClass, Kind, Method, Payload, Problem, Receiver, TypeDef, tilde_to_angle,
};

/// The stubs of a diagram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stubs {
    /// The Rust, unformatted but valid (syn parses it).
    pub code: String,
    /// Types the diagram uses but neither shows nor finds in Rust's
    /// prelude, sorted: rustc will ask for each.
    pub not_shown: Vec<String>,
}

/// The attribute every stub with parameters gets.
pub const EXPECT_UNUSED: &str = "#[expect(unused_variables, reason = \"a stub from a diagram\")]";

/// Names a stub can use without an import or a definition.
const PRELUDE: &[&str] = &[
    "bool",
    "char",
    "str",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "f32",
    "f64",
    "String",
    "Vec",
    "Option",
    "Result",
    "Box",
    "Self",
    "Send",
    "Sync",
    "Sized",
    "Unpin",
    "Fn",
    "FnMut",
    "FnOnce",
    "Clone",
    "Copy",
    "Default",
    "Drop",
    "Eq",
    "PartialEq",
    "Ord",
    "PartialOrd",
    "AsRef",
    "AsMut",
    "Into",
    "From",
    "Iterator",
    "IntoIterator",
    "Extend",
    "ToOwned",
    "ToString",
    "TryFrom",
    "TryInto",
    "FromIterator",
];

/// The stubs of `diagram`, or every reason it cannot have them, each at
/// its line.
pub fn generate(diagram: &ClassDiagram) -> Result<Stubs, Vec<Problem>> {
    let (classes, mut problems) = model::diagram_classes(diagram);
    let code: Vec<&DiagramClass> = classes.iter().filter(|class| class.code).collect();
    problems.extend(own_problems(diagram, &code));
    let defs: Vec<&TypeDef> = code.iter().map(|class| &class.def).collect();
    let lookup = |name: &str| defs.iter().copied().find(|def| def.name == name);
    problems.extend(model::relation_problems(diagram, &classes, &lookup));
    let mut writer = Writer {
        diagram,
        code: &code,
        out: String::new(),
        names: BTreeSet::new(),
        problems,
    };
    for class in &code {
        writer.class(class);
    }
    let Writer {
        out,
        names,
        mut problems,
        ..
    } = writer;
    if !problems.is_empty() {
        problems.sort_by_key(|problem| problem.line);
        problems.dedup();
        return Err(problems);
    }
    if let Err(err) = syn::parse_file(&out) {
        return Err(vec![problem(
            1,
            format!("the stubs do not parse as Rust ({err}); this is a bug in diagram-stubs"),
        )]);
    }
    let shown: BTreeSet<&str> = code
        .iter()
        .map(|class| class.def.name.as_str())
        .chain(
            code.iter()
                .flat_map(|class| class.def.generics.iter().map(String::as_str)),
        )
        .collect();
    let not_shown = names
        .into_iter()
        .filter(|name| !shown.contains(name.as_str()) && !PRELUDE.contains(&name.as_str()))
        .collect();
    Ok(Stubs {
        code: out,
        not_shown,
    })
}

fn problem(line: usize, message: impl Into<String>) -> Problem {
    Problem {
        line,
        message: message.into(),
    }
}

/// What the stubs refuse beyond the check's rules: excerpts, names twice,
/// unused type parameters, function classes without their function,
/// realizations the stubs cannot write.
fn own_problems(diagram: &ClassDiagram, code: &[&DiagramClass]) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut free_functions: Vec<(&str, usize)> = Vec::new();
    for class in code {
        let def = &class.def;
        let name = &def.name;
        if class.excerpt {
            problems.push(problem(
                def.line,
                format!(
                    "class `{name}` is an <<excerpt>>; stubs of part of a type are not the type"
                ),
            ));
        }
        twice(
            def.fields.iter().map(|f| (f.name.as_str(), f.line)),
            "field",
            name,
            &mut problems,
        );
        twice(
            def.variants.iter().map(|v| (v.name.as_str(), v.line)),
            "variant",
            name,
            &mut problems,
        );
        twice(
            def.methods.iter().map(|m| (m.name.as_str(), m.line)),
            "method",
            name,
            &mut problems,
        );
        if matches!(def.kind, Kind::Struct | Kind::Enum) {
            for generic in &def.generics {
                if !def.held().iter().any(|(_, ty)| ty.names(generic)) {
                    problems.push(problem(
                        def.line,
                        format!(
                            "class `{name}`'s type parameter `{generic}` is used by no field; rustc refuses a parameter nothing uses"
                        ),
                    ));
                }
            }
        }
        if def.kind == Kind::Function {
            if !def.generics.is_empty() {
                problems.push(problem(
                    def.line,
                    format!("the <<function>> class `{name}` has no type parameters to give"),
                ));
            }
            if !def.methods.iter().any(|method| method.name == *name) {
                problems.push(problem(
                    def.line,
                    format!(
                        "a <<function>> class is named after one of its functions; `{name}` shows none of that name"
                    ),
                ));
            }
            for method in &def.methods {
                free_functions.push((method.name.as_str(), method.line));
            }
        }
        if !def.implements.is_empty() && matches!(def.kind, Kind::Trait | Kind::Function) {
            problems.push(problem(
                def.line,
                format!(
                    "`{name}` is {}; only a struct or an enum implements a trait here",
                    def.kind
                ),
            ));
        }
        for implemented in &def.implements {
            let generic = code
                .iter()
                .find(|other| other.def.name == *implemented)
                .is_some_and(|other| !other.def.generics.is_empty());
            if generic {
                let line = diagram
                    .relations
                    .iter()
                    .find(|r| r.from == *name && r.to == *implemented)
                    .map_or(def.line, |r| r.line);
                problems.push(problem(
                    line,
                    format!(
                        "`{name} ..|> {implemented}`: the stubs implement no trait with type parameters; write that impl by hand"
                    ),
                ));
            }
        }
    }
    twice(
        free_functions.into_iter(),
        "free function",
        "the diagram",
        &mut problems,
    );
    problems
}

fn twice<'a>(
    names: impl Iterator<Item = (&'a str, usize)>,
    what: &str,
    owner: &str,
    problems: &mut Vec<Problem>,
) {
    let mut seen = BTreeSet::new();
    for (name, line) in names {
        if !seen.insert(name) {
            problems.push(problem(
                line,
                format!("{owner} declares the {what} `{name}` twice"),
            ));
        }
    }
}

/// Writes the items, collecting the names the types use and what cannot be
/// written.
struct Writer<'a> {
    diagram: &'a ClassDiagram,
    code: &'a [&'a DiagramClass],
    out: String,
    names: BTreeSet<String>,
    problems: Vec<Problem>,
}

impl Writer<'_> {
    fn class(&mut self, class: &DiagramClass) {
        let def = &class.def;
        let generics = angle(&def.generics);
        match def.kind {
            Kind::Struct => {
                self.doc(class);
                self.struct_item(def, &generics);
                self.inherent(def, &generics);
                self.realizations(def, &generics);
            }
            Kind::Enum => {
                self.doc(class);
                self.enum_item(def, &generics);
                self.inherent(def, &generics);
                self.realizations(def, &generics);
            }
            Kind::Trait => {
                self.doc(class);
                self.out
                    .push_str(&format!("pub trait {}{generics} {{\n", def.name));
                for method in &def.methods {
                    let signature = self.signature(method, true);
                    if method.is_abstract {
                        self.out.push_str(&format!("    {signature};\n"));
                    } else {
                        // A default body that takes `self`, a parameter
                        // or returns `Self` by value needs `Self` sized.
                        let by_value = method.receiver == Some(Receiver::Value)
                            || method
                                .params
                                .iter()
                                .filter_map(|param| param.ty.as_deref())
                                .any(self_by_value)
                            || method.ret.as_deref().is_some_and(self_by_value);
                        let signature = if by_value {
                            format!("{signature} where Self: Sized")
                        } else {
                            signature
                        };
                        self.body("    ", method, &signature);
                    }
                }
                self.out.push_str("}\n\n");
            }
            Kind::Function => {
                for method in &def.methods {
                    let signature = self.signature(method, false);
                    let signature = format!("{}{signature}", method.vis.keyword());
                    if let Some(note) = &class.note
                        && method.name == def.name
                    {
                        self.out.push_str(&format!("/// {}\n", one_line(note)));
                    }
                    self.body("", method, &signature);
                    self.out.push('\n');
                }
            }
        }
    }

    fn doc(&mut self, class: &DiagramClass) {
        if let Some(note) = &class.note {
            self.out.push_str(&format!("/// {}\n", one_line(note)));
        }
    }

    fn member(&self, class: &str, line: usize) -> Option<&Member> {
        self.diagram
            .class(class)?
            .members
            .iter()
            .find(|member| member.line == line)
    }

    /// The type as written, after `~` became `<>`, checked by syn.
    fn ty(&mut self, raw: &str, line: usize) -> String {
        let raw = tilde_to_angle(raw.trim());
        match syn::parse_str::<syn::Type>(&raw) {
            Ok(ty) => bare_names(&ty, &mut self.names),
            Err(_) => self
                .problems
                .push(problem(line, format!("`{raw}` is no Rust type"))),
        }
        raw
    }

    fn struct_item(&mut self, def: &TypeDef, generics: &str) {
        let name = &def.name;
        if def.fields.is_empty() {
            self.out
                .push_str(&format!("pub struct {name}{generics};\n\n"));
            return;
        }
        let mut fields = Vec::new();
        for field in &def.fields {
            let raw = self
                .member(name, field.line)
                .and_then(|member| field_type(&member.text))
                .unwrap_or_else(|| field.ty.text.clone());
            let ty = self.ty(&raw, field.line);
            fields.push((field.vis.keyword(), field.name.clone(), ty));
        }
        if def.tuple {
            let parts: Vec<String> = fields
                .iter()
                .map(|(vis, _, ty)| format!("{vis}{ty}"))
                .collect();
            self.out.push_str(&format!(
                "pub struct {name}{generics}({});\n\n",
                parts.join(", ")
            ));
        } else {
            self.out
                .push_str(&format!("pub struct {name}{generics} {{\n"));
            for (vis, field, ty) in fields {
                self.out.push_str(&format!("    {vis}{field}: {ty},\n"));
            }
            self.out.push_str("}\n\n");
        }
    }

    fn enum_item(&mut self, def: &TypeDef, generics: &str) {
        let name = &def.name;
        self.out
            .push_str(&format!("pub enum {name}{generics} {{\n"));
        for variant in &def.variants {
            let parts = self
                .member(name, variant.line)
                .map(|member| payload_parts(&member.text))
                .unwrap_or_default();
            match &variant.payload {
                Payload::Unit => self.out.push_str(&format!("    {},\n", variant.name)),
                Payload::Tuple(_) => {
                    let types: Vec<String> = parts
                        .iter()
                        .map(|part| self.ty(part, variant.line))
                        .collect();
                    self.out
                        .push_str(&format!("    {}({}),\n", variant.name, types.join(", ")));
                }
                Payload::Named(_) => {
                    let mut fields = Vec::new();
                    for part in &parts {
                        let (field, ty) = part.split_once(':').unwrap_or((part, ""));
                        let ty = self.ty(ty, variant.line);
                        fields.push(format!("{}: {ty}", field.trim()));
                    }
                    self.out.push_str(&format!(
                        "    {} {{ {} }},\n",
                        variant.name,
                        fields.join(", ")
                    ));
                }
                Payload::Unshown => self.problems.push(problem(
                    variant.line,
                    format!(
                        "the variant `{}(…)` does not show its payload; the stubs need its types",
                        variant.name
                    ),
                )),
            }
        }
        self.out.push_str("}\n\n");
    }

    fn inherent(&mut self, def: &TypeDef, generics: &str) {
        if def.methods.is_empty() {
            return;
        }
        self.out
            .push_str(&format!("impl{generics} {}{generics} {{\n", def.name));
        for method in &def.methods {
            let signature = self.signature(method, true);
            let signature = format!("{}{signature}", method.vis.keyword());
            self.body("    ", method, &signature);
        }
        self.out.push_str("}\n\n");
    }

    fn realizations(&mut self, def: &TypeDef, generics: &str) {
        for implemented in &def.implements {
            let Some(target) = self
                .code
                .iter()
                .find(|class| class.def.name == *implemented && class.def.kind == Kind::Trait)
            else {
                continue;
            };
            self.out.push_str(&format!(
                "impl{generics} {implemented} for {}{generics} {{\n",
                def.name
            ));
            for method in target.def.methods.iter().filter(|m| m.is_abstract) {
                let signature = self.signature(method, true);
                self.body("    ", method, &signature);
            }
            self.out.push_str("}\n\n");
        }
    }

    /// `fn name(receiver, params) -> Return`, without visibility.
    fn signature(&mut self, method: &Method, default_receiver: bool) -> String {
        let mut parts = Vec::new();
        let receiver = match &method.receiver {
            Some(Receiver::None) => None,
            Some(Receiver::Value) => Some("self".to_owned()),
            Some(Receiver::Ref) => Some("&self".to_owned()),
            Some(Receiver::RefMut) => Some("&mut self".to_owned()),
            Some(Receiver::Typed(ty)) => Some(format!("self: {ty}")),
            None => default_receiver.then(|| "&self".to_owned()),
        };
        parts.extend(receiver);
        for param in &method.params {
            match (&param.name, &param.ty) {
                (Some(name), Some(ty)) if crate::mermaid::is_ident(name.trim()) => {
                    let ty = self.ty(ty, method.line);
                    parts.push(format!("{}: {ty}", name.trim()));
                }
                _ => self.problems.push(problem(
                    method.line,
                    format!(
                        "the parameter `{}` of `{}` is written `name: Type` for stubs",
                        param.text, method.name
                    ),
                )),
            }
        }
        let ret = match &method.ret {
            Some(ret) => format!(" -> {}", self.ty(ret, method.line)),
            None => String::new(),
        };
        format!("fn {}({}){ret}", method.name, parts.join(", "))
    }

    fn body(&mut self, indent: &str, method: &Method, signature: &str) {
        // rustc warns of an unused parameter only when its name does not
        // start with `_`; an `expect` without that warning is itself one.
        let warns = method.params.iter().any(|param| {
            param
                .name
                .as_deref()
                .is_some_and(|name| !name.trim().starts_with('_'))
        });
        if warns {
            self.out.push_str(&format!("{indent}{EXPECT_UNUSED}\n"));
        }
        self.out.push_str(&format!(
            "{indent}{signature} {{\n{indent}    todo!()\n{indent}}}\n"
        ));
    }
}

fn angle(generics: &[String]) -> String {
    if generics.is_empty() {
        String::new()
    } else {
        format!("<{}>", generics.join(", "))
    }
}

/// Whether a type, as a diagram writes it, holds `Self` by value: not
/// behind `&` or `*`, so that a default body needs `Self: Sized`
/// (`Self`, `Option<Self>`, `Vec<Self>`, `(Self, u8)`, but not `&Self`).
fn self_by_value(text: &str) -> bool {
    match syn::parse_str::<syn::Type>(&tilde_to_angle(text.trim())) {
        Ok(ty) => holds_self(&ty),
        // Not a type: `signature` refuses it; be careful meanwhile.
        Err(_) => text
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .any(|word| word == "Self"),
    }
}

fn holds_self(ty: &syn::Type) -> bool {
    use syn::Type;
    match ty {
        Type::Path(path) => {
            if path.qself.is_none()
                && path.path.segments.len() == 1
                && path.path.segments[0].ident == "Self"
            {
                return true;
            }
            path.path
                .segments
                .iter()
                .any(|segment| match &segment.arguments {
                    syn::PathArguments::AngleBracketed(arguments) => {
                        arguments.args.iter().any(|argument| match argument {
                            syn::GenericArgument::Type(inner) => holds_self(inner),
                            _ => false,
                        })
                    }
                    _ => false,
                })
        }
        Type::Reference(_) | Type::Ptr(_) => false,
        Type::Slice(slice) => holds_self(&slice.elem),
        Type::Array(array) => holds_self(&array.elem),
        Type::Tuple(tuple) => tuple.elems.iter().any(holds_self),
        Type::Paren(paren) => holds_self(&paren.elem),
        Type::Group(group) => holds_self(&group.elem),
        _ => false,
    }
}

fn one_line(text: &str) -> String {
    text.replace("<br>", " ")
        .replace("<br/>", " ")
        .replace("\\n", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A field line's type as written: `+Vec~String~ labels` gives
/// `Vec~String~`.
fn field_type(text: &str) -> Option<String> {
    let rest = text.get(1..)?.trim_start();
    let (ty, _name) = rest.rsplit_once(char::is_whitespace)?;
    Some(ty.trim().to_owned())
}

/// A variant line's payload, split at its top-level commas, `~` turned
/// into `<>`.
fn payload_parts(text: &str) -> Vec<String> {
    let (Some(open), Some(close)) = (text.find('('), text.rfind(')')) else {
        return Vec::new();
    };
    let inner = tilde_to_angle(&text[open + 1..close]);
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for c in inner.chars() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            _ => {}
        }
        if c == ',' && depth == 0 {
            parts.push(std::mem::take(&mut current).trim().to_owned());
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }
    parts
}

/// The names a type uses without a path: `Vec<RepoName>` gives `Vec` and
/// `RepoName`; `std::path::PathBuf` gives nothing, it needs no import.
fn bare_names(ty: &syn::Type, out: &mut BTreeSet<String>) {
    use syn::Type;
    match ty {
        Type::Path(path) => {
            if let Some(qself) = &path.qself {
                bare_names(&qself.ty, out);
            }
            path_names(&path.path, out);
        }
        Type::Reference(reference) => bare_names(&reference.elem, out),
        Type::Slice(slice) => bare_names(&slice.elem, out),
        Type::Array(array) => bare_names(&array.elem, out),
        Type::Ptr(ptr) => bare_names(&ptr.elem, out),
        Type::Paren(paren) => bare_names(&paren.elem, out),
        Type::Group(group) => bare_names(&group.elem, out),
        Type::Tuple(tuple) => tuple.elems.iter().for_each(|elem| bare_names(elem, out)),
        Type::TraitObject(object) => bounds(&object.bounds, out),
        Type::ImplTrait(implementation) => bounds(&implementation.bounds, out),
        _ => {}
    }
}

fn bounds(
    bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    out: &mut BTreeSet<String>,
) {
    for bound in bounds {
        if let syn::TypeParamBound::Trait(bound) = bound {
            path_names(&bound.path, out);
        }
    }
}

fn path_names(path: &syn::Path, out: &mut BTreeSet<String>) {
    if path.segments.len() == 1 && path.leading_colon.is_none() {
        out.insert(path.segments[0].ident.to_string());
    }
    for segment in &path.segments {
        match &segment.arguments {
            syn::PathArguments::AngleBracketed(arguments) => {
                for argument in &arguments.args {
                    match argument {
                        syn::GenericArgument::Type(ty) => bare_names(ty, out),
                        syn::GenericArgument::AssocType(assoc) => bare_names(&assoc.ty, out),
                        _ => {}
                    }
                }
            }
            syn::PathArguments::Parenthesized(arguments) => {
                for input in &arguments.inputs {
                    bare_names(&input.ty, out);
                }
                if let syn::ReturnType::Type(_, ty) = &arguments.output {
                    bare_names(ty, out);
                }
            }
            syn::PathArguments::None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::class::parse;

    fn stubs(text: &str) -> Result<Stubs, Vec<Problem>> {
        generate(&parse(text).unwrap())
    }

    #[test]
    fn a_trait_and_its_realization() {
        let stubs = stubs(
            "classDiagram
class Run {
  <<trait>>
  +start(&mut self, port: u16)*
  +stop(self)
}
class Vm {
  -u16 port
}
Vm ..|> Run
",
        )
        .unwrap();
        assert!(
            stubs
                .code
                .contains("pub trait Run {\n    fn start(&mut self, port: u16);\n"),
            "{}",
            stubs.code
        );
        assert!(
            stubs
                .code
                .contains("    fn stop(self) where Self: Sized {\n        todo!()\n    }\n")
        );
        assert!(
            stubs
                .code
                .contains("impl Run for Vm {\n    #[expect(unused_variables")
        );
        assert!(
            !stubs.code.contains("impl Run for Vm {\n    fn stop"),
            "a default is not repeated"
        );
    }

    #[test]
    fn names_without_a_path_that_are_not_shown_are_listed() {
        let stubs = stubs(
            "classDiagram
class Job {
  +RepoName repo
  +std::path::PathBuf dir
  +Vec~Step~ steps
  +run(&self, at: Instant) Result~Step, Error~
}
class Step
",
        )
        .unwrap();
        assert_eq!(stubs.not_shown, ["Error", "Instant", "RepoName"]);
    }

    #[test]
    fn a_free_function_and_its_note() {
        let stubs = stubs(
            "classDiagram
class qualify {
  <<function>>
  +qualify(ticket: &str) bool
  -helper()
}
note for qualify \"applies the rules\"
",
        )
        .unwrap();
        assert!(
            stubs
                .code
                .contains("/// applies the rules\n#[expect(unused_variables"),
            "{}",
            stubs.code
        );
        assert!(
            stubs
                .code
                .contains("pub fn qualify(ticket: &str) -> bool {")
        );
        assert!(stubs.code.contains("\nfn helper() {"));
    }
}
