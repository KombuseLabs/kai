//! What a class says, from the diagram's side and from the code's, in one
//! shape: [`TypeDef`]. The diagram's side comes from [`diagram_classes`],
//! the code's from [`crate::rust`]; [`compare`] and [`relation_problems`]
//! are the rules both the check and the stubs keep.
//!
//! How a class diagram maps to Rust:
//!
//! | In the diagram | In Rust |
//! | --- | --- |
//! | a class without a stereotype | a `struct` |
//! | `<<enumeration>>` | an `enum`; `Name` a unit variant, `Name(T, U)` a tuple variant, `Name(a: T)` a variant with named fields, `Name(…)` a payload the diagram does not show |
//! | `<<trait>>` | a `trait`; a method marked `*` has no default body |
//! | `<<function>>` | its methods are free functions of one module |
//! | `<<excerpt>>`, besides one of the above | the class shows part of its fields or variants on purpose |
//! | any other stereotype | not code |
//! | `+`, `~`, `-` | `pub`, `pub(crate)` or `pub(super)`, private |
//! | `+Type name` | a field |
//! | `+name(params) Return` | a method; `$` after it: no receiver; a first parameter `self`, `&self` or `&mut self`: that receiver |
//! | `Name~T~`, `Vec~String~` | `Name<T>`, `Vec<String>` |
//!
//! Types are compared after each path is cut to its last segment
//! (`std::time::Duration` is `Duration`), lifetimes dropped and whitespace
//! removed.
//!
//! ```
//! use diagrams::model::Ty;
//!
//! let shown = Ty::from_diagram("BTreeMap~String, Vec~u64~~").unwrap();
//! let code = Ty::from_diagram("std::collections::BTreeMap<String, Vec<u64>>").unwrap();
//! assert_eq!(shown, code);
//! assert_eq!(shown.text, "BTreeMap<String, Vec<u64>>");
//! ```

use std::collections::BTreeSet;
use std::fmt;

use quote::ToTokens;

use crate::mermaid::class::{ClassDiagram, Member, RelationKind};

/// A visibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vis {
    /// `pub`, `+`.
    Pub,
    /// `pub(crate)`, `pub(super)` or `pub(in …)`, `~`.
    Crate,
    /// Private, `-`.
    Private,
}

impl Vis {
    /// The mark a diagram writes for it.
    pub fn mark(self) -> char {
        match self {
            Vis::Pub => '+',
            Vis::Crate => '~',
            Vis::Private => '-',
        }
    }

    /// The keyword Rust writes for it (empty for private).
    pub fn keyword(self) -> &'static str {
        match self {
            Vis::Pub => "pub ",
            Vis::Crate => "pub(crate) ",
            Vis::Private => "",
        }
    }
}

/// How a method takes `self`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// No receiver: an associated or free function (`$` in a diagram).
    None,
    /// `self` or `mut self`.
    Value,
    /// `&self`.
    Ref,
    /// `&mut self`.
    RefMut,
    /// `self: Type`, by the type's canonical form.
    Typed(String),
}

impl fmt::Display for Receiver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Receiver::None => f.write_str("no self"),
            Receiver::Value => f.write_str("self"),
            Receiver::Ref => f.write_str("&self"),
            Receiver::RefMut => f.write_str("&mut self"),
            Receiver::Typed(ty) => write!(f, "self: {ty}"),
        }
    }
}

/// How many of something a field holds, by the outside of its type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A plain type: multiplicity `1`.
    One,
    /// An `Option`: `0..1`.
    Optional,
    /// A `Vec`, a map, a set, a slice or an array: `*`, `0..*`, `1..*`.
    Many,
}

impl Shape {
    /// The shape a multiplicity stands for.
    pub fn of_multiplicity(multiplicity: &str) -> Option<Shape> {
        match multiplicity.trim() {
            "1" => Some(Shape::One),
            "0..1" => Some(Shape::Optional),
            "*" | "0..*" | "1..*" | "n" | "many" => Some(Shape::Many),
            _ => None,
        }
    }
}

/// A Rust type, as shown and in canonical form.
#[derive(Clone, Debug)]
pub struct Ty {
    /// For people: the canonical form with a space after each comma.
    pub text: String,
    /// What is compared.
    pub canon: String,
    /// The shape of its outside.
    pub shape: Shape,
    /// Every name in it (the last segment of each path).
    pub names: BTreeSet<String>,
}

impl PartialEq for Ty {
    fn eq(&self, other: &Ty) -> bool {
        self.canon == other.canon
    }
}

impl Eq for Ty {}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Collections a field can hold many of.
const COLLECTIONS: &[&str] = &[
    "Vec",
    "VecDeque",
    "LinkedList",
    "HashMap",
    "BTreeMap",
    "HashSet",
    "BTreeSet",
    "BinaryHeap",
    "IndexMap",
    "IndexSet",
];

/// Wrappers that do not change how many a field holds.
const WRAPPERS: &[&str] = &[
    "Box", "Arc", "Rc", "Cow", "Mutex", "RwLock", "RefCell", "Cell",
];

impl Ty {
    /// A type as a diagram writes it: `Vec~String~` is `Vec<String>`.
    pub fn from_diagram(text: &str) -> Result<Ty, String> {
        let rust = tilde_to_angle(text.trim());
        let parsed: syn::Type =
            syn::parse_str(&rust).map_err(|_| format!("`{}` is no Rust type", text.trim()))?;
        Ok(Ty::from_syn(&parsed))
    }

    /// A type syn read.
    pub fn from_syn(ty: &syn::Type) -> Ty {
        let mut names = BTreeSet::new();
        let canon = canon(ty, &mut names);
        Ty {
            text: canon.replace(',', ", "),
            shape: shape(ty),
            canon,
            names,
        }
    }

    /// Whether the type names `name` anywhere.
    pub fn names(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}

/// Turns Mermaid's generics into Rust's: a `~` right after a name and
/// right before a type opens (`<`), any other closes (`>`).
///
/// ```
/// use diagrams::model::tilde_to_angle;
///
/// assert_eq!(tilde_to_angle("Option~Vec~String~~"), "Option<Vec<String>>");
/// assert_eq!(tilde_to_angle("BTreeMap~String, u64~ map"), "BTreeMap<String, u64> map");
/// assert_eq!(tilde_to_angle("Box~dyn Error~"), "Box<dyn Error>");
/// ```
pub fn tilde_to_angle(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (index, &c) in chars.iter().enumerate() {
        if c != '~' {
            out.push(c);
            continue;
        }
        let before = index
            .checked_sub(1)
            .and_then(|i| chars.get(i))
            .is_some_and(|c| c.is_alphanumeric() || *c == '_');
        let after = chars
            .get(index + 1)
            .is_some_and(|c| c.is_alphanumeric() || "_&(['*".contains(*c));
        out.push(if before && after { '<' } else { '>' });
    }
    out
}

fn tokens(item: &impl ToTokens) -> String {
    item.to_token_stream().to_string()
}

/// The canonical form of a type: each path cut to its last segment,
/// lifetimes dropped, no whitespace but inside `dyn`, `impl`, `mut` and
/// `const`.
fn canon(ty: &syn::Type, names: &mut BTreeSet<String>) -> String {
    use syn::Type;
    match ty {
        Type::Path(path) if path.qself.is_none() => canon_path(&path.path, names),
        Type::Reference(reference) => {
            let inner = canon(&reference.elem, names);
            if reference.mutability.is_some() {
                format!("&mut {inner}")
            } else {
                format!("&{inner}")
            }
        }
        Type::Slice(slice) => format!("[{}]", canon(&slice.elem, names)),
        Type::Array(array) => format!(
            "[{};{}]",
            canon(&array.elem, names),
            tokens(&array.len).replace(' ', "")
        ),
        Type::Tuple(tuple) => {
            let parts: Vec<String> = tuple.elems.iter().map(|t| canon(t, names)).collect();
            format!("({})", parts.join(","))
        }
        Type::Ptr(ptr) => {
            let inner = canon(&ptr.elem, names);
            if matches!(ptr.mutability, syn::PointerMutability::Mut(_)) {
                format!("*mut {inner}")
            } else {
                format!("*const {inner}")
            }
        }
        Type::Paren(paren) => canon(&paren.elem, names),
        Type::Group(group) => canon(&group.elem, names),
        Type::TraitObject(object) => format!("dyn {}", canon_bounds(&object.bounds, names)),
        Type::ImplTrait(implementation) => {
            format!("impl {}", canon_bounds(&implementation.bounds, names))
        }
        Type::Never(_) => "!".to_owned(),
        Type::Infer(_) => "_".to_owned(),
        other => tokens(other).replace(' ', ""),
    }
}

fn canon_bounds(
    bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    names: &mut BTreeSet<String>,
) -> String {
    let parts: Vec<String> = bounds
        .iter()
        .filter_map(|bound| match bound {
            syn::TypeParamBound::Trait(bound) => Some(canon_path(&bound.path, names)),
            _ => None,
        })
        .collect();
    parts.join("+")
}

fn canon_path(path: &syn::Path, names: &mut BTreeSet<String>) -> String {
    let Some(last) = path.segments.last() else {
        return tokens(path).replace(' ', "");
    };
    let name = last.ident.to_string();
    names.insert(name.clone());
    match &last.arguments {
        syn::PathArguments::None => name,
        syn::PathArguments::AngleBracketed(arguments) => {
            let parts: Vec<String> = arguments
                .args
                .iter()
                .filter_map(|argument| match argument {
                    syn::GenericArgument::Lifetime(_) => None,
                    syn::GenericArgument::Type(ty) => Some(canon(ty, names)),
                    syn::GenericArgument::AssocType(assoc) => {
                        Some(format!("{}={}", assoc.ident, canon(&assoc.ty, names)))
                    }
                    other => Some(tokens(other).replace(' ', "")),
                })
                .collect();
            if parts.is_empty() {
                name
            } else {
                format!("{name}<{}>", parts.join(","))
            }
        }
        syn::PathArguments::Parenthesized(arguments) => {
            let inputs: Vec<String> = arguments
                .inputs
                .iter()
                .map(|t| canon(&t.ty, names))
                .collect();
            let output = match &arguments.output {
                syn::ReturnType::Default => String::new(),
                syn::ReturnType::Type(_, ty) => format!("->{}", canon(ty, names)),
            };
            format!("{name}({}){output}", inputs.join(","))
        }
    }
}

fn shape(ty: &syn::Type) -> Shape {
    use syn::Type;
    match ty {
        Type::Slice(_) | Type::Array(_) => Shape::Many,
        Type::Reference(reference) => shape(&reference.elem),
        Type::Paren(paren) => shape(&paren.elem),
        Type::Group(group) => shape(&group.elem),
        Type::Path(path) => {
            let Some(last) = path.path.segments.last() else {
                return Shape::One;
            };
            let name = last.ident.to_string();
            if name == "Option" {
                Shape::Optional
            } else if COLLECTIONS.contains(&name.as_str()) {
                Shape::Many
            } else if WRAPPERS.contains(&name.as_str()) {
                match &last.arguments {
                    syn::PathArguments::AngleBracketed(arguments) => arguments
                        .args
                        .iter()
                        .find_map(|argument| match argument {
                            syn::GenericArgument::Type(inner) => Some(shape(inner)),
                            _ => None,
                        })
                        .unwrap_or(Shape::One),
                    _ => Shape::One,
                }
            } else {
                Shape::One
            }
        }
        _ => Shape::One,
    }
}

/// Where something is in the code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    /// The file, relative to the repository root.
    pub file: String,
    /// The line, from 1.
    pub line: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

/// A field of a struct, or of a variant with named fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// Its name; a tuple struct's fields are `0`, `1`, ….
    pub name: String,
    /// Its visibility.
    pub vis: Vis,
    /// Its type.
    pub ty: Ty,
    /// The diagram's line, or the code's.
    pub line: usize,
}

/// What a variant carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    /// Nothing: `Name`.
    Unit,
    /// `Name(T, U)`.
    Tuple(Vec<Ty>),
    /// `Name(a: T, b: U)`; in Rust, `Name { a: T, b: U }`.
    Named(Vec<(String, Ty)>),
    /// `Name(…)`: the diagram does not show it (diagrams only).
    Unshown,
}

impl fmt::Display for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Payload::Unit => Ok(()),
            Payload::Tuple(types) => {
                let parts: Vec<String> = types.iter().map(|ty| ty.text.clone()).collect();
                write!(f, "({})", parts.join(", "))
            }
            Payload::Named(fields) => {
                let parts: Vec<String> = fields
                    .iter()
                    .map(|(name, ty)| format!("{name}: {ty}"))
                    .collect();
                write!(f, "({})", parts.join(", "))
            }
            Payload::Unshown => f.write_str("(…)"),
        }
    }
}

/// A variant of an enum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    /// Its name.
    pub name: String,
    /// What it carries.
    pub payload: Payload,
    /// The diagram's line, or the code's.
    pub line: usize,
}

/// A parameter other than the receiver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    /// Its name, if written.
    pub name: Option<String>,
    /// Its type, if written (Rust syntax).
    pub ty: Option<String>,
    /// As written.
    pub text: String,
}

/// A method of a type, an item of a trait, or a free function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Method {
    /// Its name.
    pub name: String,
    /// Its visibility; the items of traits and of trait impls are `Pub`.
    pub vis: Vis,
    /// How it takes `self`; `None` where a diagram does not say.
    pub receiver: Option<Receiver>,
    /// For a trait's item: whether it has no default body (`*`).
    pub is_abstract: bool,
    /// Its parameters, without the receiver.
    pub params: Vec<Param>,
    /// Its return type, as written (Rust syntax), if any.
    pub ret: Option<String>,
    /// The trait whose impl holds it (code only).
    pub from_trait: Option<String>,
    /// The diagram's line, or the code's.
    pub line: usize,
    /// Where it is (code only).
    pub location: Option<Location>,
    /// Its stamp (code only).
    pub stamp: Option<String>,
}

/// What a class stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A `struct`.
    Struct,
    /// An `enum`.
    Enum,
    /// A `trait`.
    Trait,
    /// Free functions.
    Function,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Struct => "a struct",
            Kind::Enum => "an enum",
            Kind::Trait => "a trait",
            Kind::Function => "a function",
        })
    }
}

impl Kind {
    /// The stereotype a diagram writes for it.
    pub fn stereotype(self) -> &'static str {
        match self {
            Kind::Struct => "no stereotype",
            Kind::Enum => "<<enumeration>>",
            Kind::Trait => "<<trait>>",
            Kind::Function => "<<function>>",
        }
    }
}

/// A struct, an enum, a trait or a function, as a diagram or the code says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeDef {
    /// Its name.
    pub name: String,
    /// What it is.
    pub kind: Kind,
    /// Its type parameters, by name.
    pub generics: Vec<String>,
    /// A struct's fields.
    pub fields: Vec<Field>,
    /// Whether a struct is a tuple struct.
    pub tuple: bool,
    /// An enum's variants.
    pub variants: Vec<Variant>,
    /// Methods, trait items, or for a function its module's free functions.
    pub methods: Vec<Method>,
    /// The traits it implements, by name.
    pub implements: Vec<String>,
    /// The diagram's line, or the code's.
    pub line: usize,
    /// Where it is (code only).
    pub location: Option<Location>,
    /// Its stamp (code only).
    pub stamp: Option<String>,
}

impl TypeDef {
    /// An empty definition.
    pub fn new(name: &str, kind: Kind, line: usize) -> TypeDef {
        TypeDef {
            name: name.to_owned(),
            kind,
            generics: Vec::new(),
            fields: Vec::new(),
            tuple: false,
            variants: Vec::new(),
            methods: Vec::new(),
            implements: Vec::new(),
            line,
            location: None,
            stamp: None,
        }
    }

    /// Every type this definition holds, with the field's name: a struct's
    /// fields, an enum's payloads.
    pub fn held(&self) -> Vec<(String, &Ty)> {
        let mut held: Vec<(String, &Ty)> = self
            .fields
            .iter()
            .map(|field| (field.name.clone(), &field.ty))
            .collect();
        for variant in &self.variants {
            match &variant.payload {
                Payload::Tuple(types) => {
                    for (index, ty) in types.iter().enumerate() {
                        held.push((format!("{}.{index}", variant.name), ty));
                    }
                }
                Payload::Named(fields) => {
                    for (name, ty) in fields {
                        held.push((format!("{}.{name}", variant.name), ty));
                    }
                }
                Payload::Unit | Payload::Unshown => {}
            }
        }
        held
    }

    fn at(&self) -> String {
        self.location
            .as_ref()
            .map(|location| format!(" ({location})"))
            .unwrap_or_default()
    }
}

/// A class of a diagram, read as Rust.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagramClass {
    /// What it says, as Rust; meaningless when `code` is false.
    pub def: TypeDef,
    /// Whether it is code (else a stereotype says it is not).
    pub code: bool,
    /// Whether it shows part of its fields or variants on purpose.
    pub excerpt: bool,
    /// Its stereotypes.
    pub stereotypes: Vec<String>,
    /// The text of its note, if it has one.
    pub note: Option<String>,
}

/// A line of a diagram that cannot be read as Rust.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The diagram's line, from 1.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl Problem {
    fn new(line: usize, message: impl Into<String>) -> Problem {
        Problem {
            line,
            message: message.into(),
        }
    }
}

/// The classes of a diagram, read as Rust, and what in them cannot be.
pub fn diagram_classes(diagram: &ClassDiagram) -> (Vec<DiagramClass>, Vec<Problem>) {
    let mut classes = Vec::new();
    let mut problems = Vec::new();
    for class in &diagram.classes {
        let mut kind = None;
        let mut excerpt = false;
        let mut other = false;
        for stereotype in &class.stereotypes {
            let this = match stereotype.as_str() {
                "enumeration" => Some(Kind::Enum),
                "trait" => Some(Kind::Trait),
                "function" => Some(Kind::Function),
                "excerpt" => {
                    excerpt = true;
                    None
                }
                _ => {
                    other = true;
                    None
                }
            };
            if let Some(this) = this {
                if kind.is_some_and(|kind| kind != this) {
                    problems.push(Problem::new(
                        class.line,
                        format!("class `{}` has two kinds of stereotype", class.name),
                    ));
                }
                kind = Some(this);
            }
        }
        let code = !other;
        let kind = kind.unwrap_or(Kind::Struct);
        let mut def = TypeDef::new(&class.name, kind, class.line);
        def.generics = class.generics.clone();
        if code {
            for member in &class.members {
                if let Err(problem) = read_member(&mut def, member) {
                    problems.push(problem);
                }
            }
            if excerpt && matches!(kind, Kind::Trait | Kind::Function) {
                problems.push(Problem::new(
                    class.line,
                    format!("`<<excerpt>>` is for a struct's fields or an enum's variants; class `{}` may show any part of its methods without it", class.name),
                ));
            }
        }
        for relation in &diagram.relations {
            if relation.kind == RelationKind::Realization && relation.from == class.name {
                def.implements.push(relation.to.clone());
            }
        }
        classes.push(DiagramClass {
            def,
            code,
            excerpt,
            stereotypes: class.stereotypes.clone(),
            note: diagram.note_for(&class.name).map(|note| note.text.clone()),
        });
    }
    (classes, problems)
}

/// Reads one member line into `def`.
fn read_member(def: &mut TypeDef, member: &Member) -> Result<(), Problem> {
    let text = member.text.as_str();
    let line = member.line;
    if def.kind == Kind::Enum {
        def.variants.push(read_variant(text, line)?);
        return Ok(());
    }
    let (vis, rest) = read_vis(text, line)?;
    if rest.contains('(') {
        let method = read_method(vis, rest, line)?;
        if method.is_abstract && def.kind != Kind::Trait {
            return Err(Problem::new(
                line,
                "`*` marks a trait's item without a default body; only a `<<trait>>` has those",
            ));
        }
        if def.kind == Kind::Function
            && method
                .receiver
                .as_ref()
                .is_some_and(|r| *r != Receiver::None)
        {
            return Err(Problem::new(line, "a free function takes no `self`"));
        }
        def.methods.push(method);
    } else {
        if def.kind != Kind::Struct {
            return Err(Problem::new(
                line,
                format!(
                    "{} has no fields; a member of this class is a method, `name(…)`",
                    def.kind
                ),
            ));
        }
        if rest.ends_with('$') {
            return Err(Problem::new(
                line,
                "a static field (`$`) has no form in Rust",
            ));
        }
        let (ty, name) = rest
            .rsplit_once(char::is_whitespace)
            .ok_or_else(|| Problem::new(line, "a field is written `+Type name`"))?;
        let name = name.trim();
        let tuple_field = name.chars().all(|c| c.is_ascii_digit());
        if !(crate::mermaid::is_ident(name) || tuple_field) {
            return Err(Problem::new(line, format!("`{name}` is no field name")));
        }
        if tuple_field {
            def.tuple = true;
        }
        let ty = Ty::from_diagram(ty).map_err(|message| Problem::new(line, message))?;
        def.fields.push(Field {
            name: name.to_owned(),
            vis,
            ty,
            line,
        });
    }
    Ok(())
}

fn read_vis(text: &str, line: usize) -> Result<(Vis, &str), Problem> {
    let mut chars = text.chars();
    let vis = match chars.next() {
        Some('+') => Vis::Pub,
        Some('~') => Vis::Crate,
        Some('-') => Vis::Private,
        Some('#') => {
            return Err(Problem::new(
                line,
                "protected (`#`) has no form in Rust; use +, ~ or -",
            ));
        }
        _ => {
            return Err(Problem::new(
                line,
                format!("`{text}`: a member starts with its visibility, +, ~ or -"),
            ));
        }
    };
    Ok((vis, chars.as_str().trim_start()))
}

/// `name(params)` with an optional `$` or `*` and an optional return type.
fn read_method(vis: Vis, text: &str, line: usize) -> Result<Method, Problem> {
    let open = text.find('(').expect("a method has a parenthesis");
    let name = text[..open].trim();
    if !crate::mermaid::is_ident(name) {
        return Err(Problem::new(
            line,
            format!(
                "`{name}` is no method name; Mermaid reads every member with `(` as a method, so a field \
                 whose type has parentheses cannot be shown: leave it out and mark the class <<excerpt>>"
            ),
        ));
    }
    let close = matching(text, open)
        .ok_or_else(|| Problem::new(line, "a method's parameters have no closing `)`"))?;
    let params_text = tilde_to_angle(&text[open + 1..close]);
    let mut rest = text[close + 1..].trim();
    let mut classifier = None;
    for mark in ['$', '*'] {
        if let Some(after) = rest.strip_prefix(mark) {
            classifier = Some(mark);
            rest = after.trim();
        } else if let Some(before) = rest.strip_suffix(mark) {
            classifier = Some(mark);
            rest = before.trim();
        }
    }
    let ret = (!rest.is_empty()).then(|| tilde_to_angle(rest));
    let mut params = Vec::new();
    let mut receiver = None;
    for (index, param) in split_top(&params_text, ',').into_iter().enumerate() {
        let param = param.trim();
        if param.is_empty() {
            continue;
        }
        if index == 0
            && let Some(written) = read_receiver(param)
        {
            receiver = Some(written);
            continue;
        }
        let (name, ty) = match split_colon(param) {
            Some((name, ty)) => (Some(name.trim().to_owned()), Some(ty.trim().to_owned())),
            None => (Some(param.to_owned()), None),
        };
        params.push(Param {
            name,
            ty,
            text: param.to_owned(),
        });
    }
    if classifier == Some('$') {
        if receiver.as_ref().is_some_and(|r| *r != Receiver::None) {
            return Err(Problem::new(line, "a method marked `$` takes no `self`"));
        }
        receiver = Some(Receiver::None);
    }
    Ok(Method {
        name: name.to_owned(),
        vis,
        receiver,
        is_abstract: classifier == Some('*'),
        params,
        ret,
        from_trait: None,
        line,
        location: None,
        stamp: None,
    })
}

fn read_receiver(param: &str) -> Option<Receiver> {
    let compact: String = param.split_whitespace().collect::<Vec<_>>().join(" ");
    match compact.as_str() {
        "self" | "mut self" => Some(Receiver::Value),
        "&self" | "& self" => Some(Receiver::Ref),
        "&mut self" | "& mut self" => Some(Receiver::RefMut),
        _ => {
            if let Some(ty) = compact.strip_prefix("self:") {
                return Some(Receiver::Typed(
                    Ty::from_diagram(ty).map_or_else(|_| ty.trim().to_owned(), |ty| ty.canon),
                ));
            }
            // `&'a self`, `&'a mut self`.
            let rest = compact.strip_prefix('&')?.trim_start();
            let rest = rest.strip_prefix('\'')?;
            let (_, after) = rest.split_once(' ')?;
            match after.trim() {
                "self" => Some(Receiver::Ref),
                "mut self" => Some(Receiver::RefMut),
                _ => None,
            }
        }
    }
}

/// A variant line: `Name`, `Name(T, U)`, `Name(a: T)` or `Name(…)`.
fn read_variant(text: &str, line: usize) -> Result<Variant, Problem> {
    if text.starts_with(['+', '-', '~', '#']) {
        return Err(Problem::new(
            line,
            "a variant is written without a visibility",
        ));
    }
    let (name, payload) = match text.find('(') {
        None => (text.trim(), Payload::Unit),
        Some(open) => {
            let close = matching(text, open)
                .ok_or_else(|| Problem::new(line, "a variant's payload has no closing `)`"))?;
            if !text[close + 1..].trim().is_empty() {
                return Err(Problem::new(
                    line,
                    "a variant is written `Name`, `Name(T, U)`, `Name(field: T)` or `Name(…)`, nothing after it",
                ));
            }
            let inner = tilde_to_angle(text[open + 1..close].trim());
            (text[..open].trim(), read_payload(&inner, line)?)
        }
    };
    if !crate::mermaid::is_ident(name) {
        return Err(Problem::new(line, format!("`{name}` is no variant name")));
    }
    Ok(Variant {
        name: name.to_owned(),
        payload,
        line,
    })
}

fn read_payload(inner: &str, line: usize) -> Result<Payload, Problem> {
    if matches!(inner, "…" | "...") {
        return Ok(Payload::Unshown);
    }
    if inner.is_empty() {
        return Err(Problem::new(
            line,
            "a variant without a payload is written `Name`",
        ));
    }
    let parts = split_top(inner, ',');
    let named: Vec<Option<(&str, &str)>> = parts.iter().map(|part| split_colon(part)).collect();
    if named.iter().all(Option::is_some) {
        let mut fields = Vec::new();
        for (name, ty) in named.into_iter().flatten() {
            let name = name.trim();
            if !crate::mermaid::is_ident(name) {
                return Err(Problem::new(line, format!("`{name}` is no field name")));
            }
            let ty = Ty::from_diagram(ty).map_err(|message| Problem::new(line, message))?;
            fields.push((name.to_owned(), ty));
        }
        Ok(Payload::Named(fields))
    } else if named.iter().all(Option::is_none) {
        let mut types = Vec::new();
        for part in parts {
            types.push(Ty::from_diagram(&part).map_err(|message| Problem::new(line, message))?);
        }
        Ok(Payload::Tuple(types))
    } else {
        Err(Problem::new(
            line,
            "a payload is all types, `(T, U)`, or all named fields, `(a: T, b: U)`",
        ))
    }
}

/// The index of the `)` that closes the `(` at `open`.
fn matching(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, c) in text.char_indices().skip_while(|(i, _)| *i < open) {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Splits at `separator` outside `<>`, `()` and `[]`.
fn split_top(text: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            _ => {}
        }
        if c == separator && depth == 0 {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() || !parts.is_empty() {
        parts.push(current);
    }
    parts
}

/// `name: Type` split at its single colon (not at `::`).
fn split_colon(text: &str) -> Option<(&str, &str)> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    for (index, &b) in bytes.iter().enumerate() {
        match b {
            b'<' | b'(' | b'[' => depth += 1,
            b'>' | b')' | b']' => depth -= 1,
            b':' if depth == 0 => {
                let double =
                    bytes.get(index + 1) == Some(&b':') || (index > 0 && bytes[index - 1] == b':');
                if !double {
                    return Some((&text[..index], &text[index + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// Where a diagram's class and the code differ, each at a line of the
/// diagram.
pub fn compare(shown: &DiagramClass, code: &TypeDef) -> Vec<Problem> {
    let mut problems = Vec::new();
    let def = &shown.def;
    let name = &def.name;
    let at = code.at();
    if def.kind != code.kind {
        problems.push(Problem::new(
            def.line,
            format!(
                "class `{name}` shows {}; `{name}` is {}{at}: mark the class {}",
                def.kind,
                code.kind,
                code.kind.stereotype()
            ),
        ));
        return problems;
    }
    if def.generics != code.generics {
        problems.push(Problem::new(
            def.line,
            format!(
                "class `{name}` shows the type parameters [{}]; the code has [{}]{at}",
                def.generics.join(", "),
                code.generics.join(", ")
            ),
        ));
    }
    match def.kind {
        Kind::Struct => compare_fields(shown, code, &mut problems),
        Kind::Enum => compare_variants(shown, code, &mut problems),
        Kind::Trait | Kind::Function => {}
    }
    compare_methods(shown, code, &mut problems);
    problems
}

fn compare_fields(shown: &DiagramClass, code: &TypeDef, problems: &mut Vec<Problem>) {
    let def = &shown.def;
    let name = &def.name;
    let file = code
        .location
        .as_ref()
        .map(|location| location.file.as_str());
    let code_at = |line: usize| {
        file.map(|file| format!(" ({file}:{line})"))
            .unwrap_or_default()
    };
    for field in &def.fields {
        match code.fields.iter().find(|f| f.name == field.name) {
            None => problems.push(Problem::new(
                field.line,
                format!(
                    "class `{name}` shows the field `{}`, which `{name}` does not have",
                    field.name
                ),
            )),
            Some(actual) => {
                if actual.vis != field.vis {
                    problems.push(Problem::new(
                        field.line,
                        format!(
                            "class `{name}` shows the field `{}` as `{}`; the code makes it `{}`{}",
                            field.name,
                            field.vis.mark(),
                            actual.vis.mark(),
                            code_at(actual.line)
                        ),
                    ));
                }
                if actual.ty != field.ty {
                    problems.push(Problem::new(
                        field.line,
                        format!(
                            "class `{name}` shows `{}: {}`; the code has `{}: {}`{}",
                            field.name,
                            field.ty,
                            actual.name,
                            actual.ty,
                            code_at(actual.line)
                        ),
                    ));
                }
            }
        }
    }
    if shown.excerpt || def.fields.is_empty() {
        return;
    }
    let shows_private = def.fields.iter().any(|field| field.vis != Vis::Pub);
    for actual in &code.fields {
        let required = actual.vis == Vis::Pub || shows_private;
        if required && !def.fields.iter().any(|field| field.name == actual.name) {
            problems.push(Problem::new(
                def.line,
                format!(
                    "class `{name}` does not show the field `{}{}: {}`{}; a class shows every public field, and every private one if it shows one, or is marked <<excerpt>>",
                    actual.vis.keyword(),
                    actual.name,
                    actual.ty,
                    code_at(actual.line)
                ),
            ));
        }
    }
}

fn compare_variants(shown: &DiagramClass, code: &TypeDef, problems: &mut Vec<Problem>) {
    let def = &shown.def;
    let name = &def.name;
    let file = code
        .location
        .as_ref()
        .map(|location| location.file.as_str());
    let code_at = |line: usize| {
        file.map(|file| format!(" ({file}:{line})"))
            .unwrap_or_default()
    };
    for variant in &def.variants {
        let Some(actual) = code.variants.iter().find(|v| v.name == variant.name) else {
            problems.push(Problem::new(
                variant.line,
                format!(
                    "class `{name}` shows the variant `{}`, which `{name}` does not have",
                    variant.name
                ),
            ));
            continue;
        };
        let differs = match (&variant.payload, &actual.payload) {
            (Payload::Unshown, Payload::Unit) => true,
            (Payload::Unshown, _) => false,
            (Payload::Named(shown), Payload::Named(actual)) => {
                let mut shown: Vec<_> = shown.iter().map(|(n, t)| (n, &t.canon)).collect();
                let mut actual: Vec<_> = actual.iter().map(|(n, t)| (n, &t.canon)).collect();
                shown.sort();
                actual.sort();
                shown != actual
            }
            (shown, actual) => shown != actual,
        };
        if differs {
            let shown_text = format!("{}{}", variant.name, variant.payload);
            let actual_text = format!("{}{}", actual.name, actual.payload);
            let hint = if variant.payload == Payload::Unshown {
                "; a variant without a payload is written without parentheses"
            } else {
                ""
            };
            problems.push(Problem::new(
                variant.line,
                format!(
                    "class `{name}` shows the variant `{shown_text}`; the code has `{actual_text}`{}{hint}",
                    code_at(actual.line)
                ),
            ));
        }
    }
    if shown.excerpt {
        return;
    }
    for actual in &code.variants {
        if !def
            .variants
            .iter()
            .any(|variant| variant.name == actual.name)
        {
            problems.push(Problem::new(
                def.line,
                format!(
                    "class `{name}` does not show the variant `{}{}`{}; an enum's class shows every variant, or is marked <<excerpt>>",
                    actual.name,
                    actual.payload,
                    code_at(actual.line)
                ),
            ));
        }
    }
}

fn compare_methods(shown: &DiagramClass, code: &TypeDef, problems: &mut Vec<Problem>) {
    let def = &shown.def;
    let name = &def.name;
    for method in &def.methods {
        let candidates: Vec<&Method> = code
            .methods
            .iter()
            .filter(|m| m.name == method.name)
            .collect();
        if candidates.is_empty() {
            let what = match def.kind {
                Kind::Function => format!(
                    "no free function `{}` is in the module of `{name}`",
                    method.name
                ),
                _ => format!("`{name}` has no method `{}`", method.name),
            };
            problems.push(Problem::new(
                method.line,
                format!(
                    "class `{name}` shows `{}(…)`; {what}{}",
                    method.name,
                    code.at()
                ),
            ));
            continue;
        }
        let mut best: Option<Vec<String>> = None;
        for actual in candidates {
            let mut differences = Vec::new();
            let where_ = actual
                .location
                .as_ref()
                .map(|location| format!(" ({location})"))
                .unwrap_or_default();
            if def.kind != Kind::Trait && actual.vis != method.vis {
                differences.push(format!(
                    "class `{name}` shows `{}` as `{}`; the code makes it `{}`{where_}",
                    method.name,
                    method.vis.mark(),
                    actual.vis.mark()
                ));
            }
            if let (Some(shown_receiver), Some(actual_receiver)) =
                (&method.receiver, &actual.receiver)
                && shown_receiver != actual_receiver
            {
                differences.push(format!(
                    "class `{name}` shows `{}` taking {shown_receiver}; the code's takes {actual_receiver}{where_}",
                    method.name
                ));
            }
            if def.kind == Kind::Trait && method.is_abstract != actual.is_abstract {
                differences.push(if method.is_abstract {
                    format!("class `{name}` marks `{}` abstract (`*`); the code gives it a default body{where_}", method.name)
                } else {
                    format!("class `{name}` shows `{}` with a default body; the code's has none{where_}: mark it `*`", method.name)
                });
            }
            if differences.is_empty() {
                best = None;
                break;
            }
            if best.as_ref().is_none_or(|b| differences.len() < b.len()) {
                best = Some(differences);
            }
        }
        for message in best.unwrap_or_default() {
            problems.push(Problem::new(method.line, message));
        }
    }
}

/// What the relations between code classes say that the definitions do
/// not hold. `lookup` gives the definition behind a class: the code's for
/// the check, the diagram's own for the stubs. Relations that touch a
/// class that is not code, or that `lookup` does not know, say nothing.
pub fn relation_problems<'a>(
    diagram: &ClassDiagram,
    classes: &[DiagramClass],
    lookup: &dyn Fn(&str) -> Option<&'a TypeDef>,
) -> Vec<Problem> {
    let is_code = |name: &str| {
        classes
            .iter()
            .any(|class| class.def.name == name && class.code)
    };
    let mut problems = Vec::new();
    for relation in &diagram.relations {
        if !is_code(&relation.from) || !is_code(&relation.to) {
            continue;
        }
        let (Some(from), Some(to)) = (lookup(&relation.from), lookup(&relation.to)) else {
            continue;
        };
        let (a, b) = (&relation.from, &relation.to);
        match relation.kind {
            RelationKind::Composition | RelationKind::Aggregation | RelationKind::Association => {
                let held: Vec<(String, &Ty)> = from
                    .held()
                    .into_iter()
                    .filter(|(_, ty)| ty.names(b))
                    .collect();
                if held.is_empty() {
                    problems.push(Problem::new(
                        relation.line,
                        format!(
                            "the relation says `{a}` holds `{b}`; no field of `{a}` names `{b}`"
                        ),
                    ));
                    continue;
                }
                if let Some(multiplicity) = &relation.to_multiplicity {
                    match Shape::of_multiplicity(multiplicity) {
                        None => problems.push(Problem::new(
                            relation.line,
                            format!("\"{multiplicity}\" is no multiplicity this check knows: \"1\", \"0..1\", \"*\", \"0..*\" or \"1..*\""),
                        )),
                        Some(shape) => {
                            if !held.iter().any(|(_, ty)| ty.shape == shape) {
                                let fields: Vec<String> = held
                                    .iter()
                                    .map(|(name, ty)| format!("`{name}: {ty}`"))
                                    .collect();
                                problems.push(Problem::new(
                                    relation.line,
                                    format!(
                                        "the multiplicity \"{multiplicity}\" at `{b}` does not fit {} of `{a}`: \"1\" is a plain type, \"0..1\" an Option, \"*\" a collection",
                                        fields.join(", ")
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
            RelationKind::Realization => {
                if to.kind != Kind::Trait {
                    problems.push(Problem::new(
                        relation.line,
                        format!("`{a} ..|> {b}` says `{a}` implements `{b}`, but `{b}` is {}, not a trait", to.kind),
                    ));
                } else if !from.implements.iter().any(|name| name == b) {
                    problems.push(Problem::new(
                        relation.line,
                        format!("`{a} ..|> {b}` says `{a}` implements `{b}`; there is no `impl {b} for {a}`{}", from.at()),
                    ));
                }
            }
            RelationKind::Link | RelationKind::Dependency | RelationKind::DashedLink => {}
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::class::parse;

    fn classes(text: &str) -> (Vec<DiagramClass>, Vec<Problem>) {
        diagram_classes(&parse(text).unwrap())
    }

    #[test]
    fn members_read_as_rust() {
        let (classes, problems) = classes(
            "classDiagram
class Egress {
  -PathBuf socket
  +BTreeMap~String, u64~ map
  +open(dir: &Path, socket: &str)$ Result~Egress~
  +port(&self) u16
  +close(self)
  ~verdict() Verdict
}
class Outcome {
  <<enumeration>>
  Pass
  Fail(String)
  Ended(how: String, stderr: String)
  Io(…)
}
class Store {
  <<trait>>
  +run()*
  +stop(&mut self)
}
",
        );
        assert!(problems.is_empty(), "{problems:?}");
        let egress = &classes[0].def;
        assert_eq!(egress.fields[1].ty.canon, "BTreeMap<String,u64>");
        assert_eq!(egress.fields[0].vis, Vis::Private);
        let open = &egress.methods[0];
        assert_eq!(open.receiver, Some(Receiver::None));
        assert_eq!(open.params[0].ty.as_deref(), Some("&Path"));
        assert_eq!(open.ret.as_deref(), Some("Result<Egress>"));
        assert_eq!(egress.methods[1].receiver, Some(Receiver::Ref));
        assert_eq!(egress.methods[2].receiver, Some(Receiver::Value));
        assert_eq!(egress.methods[3].receiver, None);
        assert_eq!(egress.methods[3].vis, Vis::Crate);
        let outcome = &classes[1].def;
        assert_eq!(outcome.variants[0].payload, Payload::Unit);
        assert!(
            matches!(&outcome.variants[1].payload, Payload::Tuple(t) if t[0].canon == "String")
        );
        assert!(matches!(&outcome.variants[2].payload, Payload::Named(f) if f.len() == 2));
        assert_eq!(outcome.variants[3].payload, Payload::Unshown);
        let store = &classes[2].def;
        assert_eq!(store.kind, Kind::Trait);
        assert!(store.methods[0].is_abstract);
        assert_eq!(store.methods[1].receiver, Some(Receiver::RefMut));
    }

    #[test]
    fn members_outside_rust_are_problems() {
        for (member, why) in [
            ("#String name", "protected"),
            ("String name", "visibility"),
            ("+String name$", "static field"),
            ("+run()*", "only a `<<trait>>`"),
            ("+not a type here x", "no Rust type"),
            ("+start(self)$", "takes no `self`"),
        ] {
            let (_, problems) = classes(&format!("classDiagram\nclass A {{\n{member}\n}}\n"));
            assert!(
                problems
                    .iter()
                    .any(|p| p.message.contains(why) && p.line == 3),
                "{member}: {problems:?}"
            );
        }
        let (_, problems) = classes("classDiagram\nclass E {\n<<enumeration>>\nA(a: u8, u8)\n}\n");
        assert!(problems[0].message.contains("all types"));
    }

    #[test]
    fn a_stereotype_that_is_not_code_is_left_alone() {
        let (classes, problems) =
            classes("classDiagram\nclass Fence {\n<<process>>\nfence proxy --listen\n}\n");
        assert!(problems.is_empty());
        assert!(!classes[0].code);
    }

    #[test]
    fn relations_are_held_by_fields() {
        let diagram = parse(
            "classDiagram
class Ticket {
  +Option~Form~ form
  +Vec~Label~ labels
}
class Form
class Label
class Run {
  <<trait>>
}
Ticket \"1\" *-- \"0..1\" Form
Ticket \"1\" *-- \"1\" Label
Ticket --> Run
Ticket ..|> Run
Form ..> Label
",
        )
        .unwrap();
        let (classes, _) = diagram_classes(&diagram);
        let mut defs: Vec<TypeDef> = classes.iter().map(|c| c.def.clone()).collect();
        defs[0].implements.clear();
        let lookup = |name: &str| defs.iter().find(|d| d.name == name);
        let problems = relation_problems(&diagram, &classes, &lookup);
        let lines: Vec<usize> = problems.iter().map(|p| p.line).collect();
        assert_eq!(lines, [12, 13, 14], "{problems:?}");
        assert!(problems[0].message.contains("\"1\" at `Label`"));
        assert!(
            problems[1]
                .message
                .contains("no field of `Ticket` names `Run`")
        );
        assert!(problems[2].message.contains("no `impl Run for Ticket`"));
    }

    #[test]
    fn types_compare_by_their_last_segments() {
        let a = Ty::from_diagram("std::time::Duration").unwrap();
        let b = Ty::from_diagram("Duration").unwrap();
        assert_eq!(a, b);
        assert_eq!(Ty::from_diagram("&'static str").unwrap().canon, "&str");
        assert_eq!(
            Ty::from_diagram("Option~Box~Form~~").unwrap().shape,
            Shape::Optional
        );
        assert_eq!(Ty::from_diagram("Arc~Vec~u8~~").unwrap().shape, Shape::Many);
        assert!(Ty::from_diagram("Vec~u8~").unwrap().names("Vec"));
    }
}
