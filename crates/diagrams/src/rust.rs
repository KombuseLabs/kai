//! A crate's source, read with syn into [`crate::model`]'s shape.
//!
//! [`CrateModel::load`] starts at `src/lib.rs` (or `src/main.rs`) and
//! follows `mod name;` to `name.rs` or `name/mod.rs`, skipping modules
//! under `#[cfg(test)]`. It keeps every struct, enum, trait and free
//! function with its module path, file and line, and every `use`. It
//! attaches each `impl` block's methods to the type its path names, seen
//! from the block's module (`impl a::X`, `impl super::X`, `impl X` after
//! `use a::X`), and stamps every item: the
//! first 8 hex digits of the SHA-256 of its tokens without doc comments, so
//! that formatting, comments and doc comments do not change a stamp and
//! code does.
//!
//! ```
//! use diagrams::rust::{CrateModel, Resolved};
//!
//! let source = "pub mod shapes { pub struct Circle { pub r: f64 } impl Circle { pub fn area(&self) -> f64 { 0.0 } } }";
//! let model = CrateModel::from_source("demo", "demo.rs", source).unwrap();
//! match model.resolve(&["shapes", "Circle", "area"]) {
//!     Resolved::Method(ty, method) => {
//!         assert_eq!(ty.def.name, "Circle");
//!         assert_eq!(method.stamp.as_ref().unwrap().len(), 8);
//!     }
//!     other => panic!("{other:?}"),
//! }
//! ```

use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::ToTokens;
use syn::spanned::Spanned;

use crate::model::{
    Field, Kind, Location, Method, Param, Payload, Receiver, Ty, TypeDef, Variant, Vis,
};

/// A struct, an enum or a trait, and the module it is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeType {
    /// The module's path from the crate's root.
    pub module: Vec<String>,
    /// What it is.
    pub def: TypeDef,
}

/// A free function, and the module it is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeFn {
    /// The module's path from the crate's root.
    pub module: Vec<String>,
    /// The function (its receiver is [`Receiver::None`]).
    pub function: Method,
}

/// What an item path names in a crate.
#[derive(Debug)]
pub enum Resolved<'a> {
    /// A module, by its path.
    Module(Vec<String>),
    /// A struct, an enum or a trait.
    Type(&'a CodeType),
    /// A free function.
    Function(&'a CodeFn),
    /// A method of a type, or an item of a trait.
    Method(&'a CodeType, &'a Method),
    /// Nothing.
    Nothing,
    /// More than one thing; the text says which.
    Ambiguous(String),
}

/// A crate, as its source says.
#[derive(Clone, Debug, Default)]
pub struct CrateModel {
    /// The crate's name: the last part of its directory.
    pub name: String,
    /// The directory, relative to the repository root.
    pub dir: String,
    /// Its structs, enums and traits.
    pub types: Vec<CodeType>,
    /// Its free functions.
    pub functions: Vec<CodeFn>,
    /// Its modules' paths, the root (`[]`) first.
    pub modules: Vec<Vec<String>>,
    /// Its `use` items.
    imports: Vec<Import>,
}

/// An `impl` block, before its methods are given to their type.
struct Implementation {
    module: Vec<String>,
    /// The path of the type, as the block writes it.
    self_path: Vec<String>,
    trait_name: Option<String>,
    methods: Vec<Method>,
}

/// A `use` item's name and path, as written in its module.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Import {
    /// The module the `use` is in.
    module: Vec<String>,
    /// The name it brings in; `None` for a glob (`use a::*`).
    name: Option<String>,
    /// The path as written: relative to the module, or from `crate`,
    /// `self` or `super`.
    path: Vec<String>,
    /// Whether it is a re-export (`pub use`, `pub(crate) use`, …).
    public: bool,
}

/// What a name in a module stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Module(Vec<String>),
    Type(usize),
    Function(usize),
    Ambiguous(String),
}

/// How many `use`s a lookup follows before it gives up (a cycle).
const MAX_DEPTH: usize = 16;

/// Reads the files of one crate.
struct Reader<'a> {
    root: &'a Path,
    model: CrateModel,
    implementations: Vec<Implementation>,
}

impl CrateModel {
    /// Reads the crate in `dir`, a directory relative to `root`.
    pub fn load(root: &Path, dir: &str) -> Result<CrateModel, String> {
        let crate_dir = root.join(dir);
        let src = crate_dir.join("src");
        let entry = ["lib.rs", "main.rs"]
            .iter()
            .map(|file| src.join(file))
            .find(|path| path.is_file())
            .ok_or_else(|| format!("{dir} has no src/lib.rs or src/main.rs"))?;
        let name = Path::new(dir)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut reader = Reader {
            root,
            model: CrateModel {
                name,
                dir: dir.to_owned(),
                modules: vec![Vec::new()],
                ..CrateModel::default()
            },
            implementations: Vec::new(),
        };
        reader.file(&entry, &[], &src)?;
        Ok(reader.finish())
    }

    /// Reads one file of source as a whole crate, with `label` as its file
    /// name in locations.
    pub fn from_source(name: &str, label: &str, source: &str) -> Result<CrateModel, String> {
        let file = syn::parse_file(source).map_err(|err| {
            let at = err.span().start();
            format!("{label}:{}: {err}", at.line)
        })?;
        let mut reader = Reader {
            root: Path::new(""),
            model: CrateModel {
                name: name.to_owned(),
                dir: String::new(),
                modules: vec![Vec::new()],
                ..CrateModel::default()
            },
            implementations: Vec::new(),
        };
        reader.items(&file.items, &[], label, None)?;
        Ok(reader.finish())
    }

    /// The structs, enums and traits of that name.
    pub fn types_named(&self, name: &str) -> Vec<&CodeType> {
        self.types.iter().filter(|ty| ty.def.name == name).collect()
    }

    /// The free functions of that name.
    pub fn functions_named(&self, name: &str) -> Vec<&CodeFn> {
        self.functions
            .iter()
            .filter(|function| function.function.name == name)
            .collect()
    }

    /// A function as a class sees it: a [`Kind::Function`] definition whose
    /// methods are the free functions of its module.
    pub fn function_def(&self, function: &CodeFn) -> TypeDef {
        let mut def = TypeDef::new(
            &function.function.name,
            Kind::Function,
            function.function.line,
        );
        def.location = function.function.location.clone();
        def.stamp = function.function.stamp.clone();
        def.methods = self
            .functions
            .iter()
            .filter(|other| other.module == function.module)
            .map(|other| other.function.clone())
            .collect();
        def
    }

    /// What `segments` (a path without the crate's name) names, as Rust
    /// resolves it from outside the crate's root: through modules, items,
    /// and re-exports (`pub use`, renamed or a glob). A path through
    /// anything else, such as a module that neither defines nor re-exports
    /// the name, names nothing; a private `use` re-exports nothing.
    pub fn resolve(&self, segments: &[&str]) -> Resolved<'_> {
        let path: Vec<String> = segments.iter().map(|s| (*s).to_owned()).collect();
        let (target, consumed) = self.resolve_from(&[], &path, false, 0);
        let rest = &path[consumed.min(path.len())..];
        match (target, rest) {
            (None, _) => Resolved::Nothing,
            (Some(Target::Ambiguous(why)), _) => Resolved::Ambiguous(why),
            (Some(Target::Module(module)), []) => Resolved::Module(module),
            (Some(Target::Type(index)), []) => Resolved::Type(&self.types[index]),
            (Some(Target::Function(index)), []) => Resolved::Function(&self.functions[index]),
            (Some(Target::Type(index)), [method_name]) => {
                let ty = &self.types[index];
                let methods: Vec<&Method> = ty
                    .def
                    .methods
                    .iter()
                    .filter(|method| method.name == *method_name)
                    .collect();
                match methods.as_slice() {
                    [method] => Resolved::Method(ty, method),
                    [] => Resolved::Nothing,
                    _ => Resolved::Ambiguous(format!(
                        "`{}` has more than one method `{method_name}` (an inherent one and a trait's?)",
                        ty.def.name
                    )),
                }
            }
            _ => Resolved::Nothing,
        }
    }

    /// Resolves `path` as written in `module`: what it names, and how many
    /// of its segments that took (a type's method is left over).
    /// `private` lets the first name be found through a private `use` of
    /// `module`, as code in that module sees it.
    fn resolve_from(
        &self,
        module: &[String],
        path: &[String],
        private: bool,
        depth: usize,
    ) -> (Option<Target>, usize) {
        if depth > MAX_DEPTH {
            return (None, 0);
        }
        let mut current = module.to_vec();
        let mut index = 0;
        match path.first().map(String::as_str) {
            Some("crate") => {
                current.clear();
                index = 1;
            }
            Some("self") => index = 1,
            _ => {}
        }
        while path.get(index).map(String::as_str) == Some("super") {
            if current.pop().is_none() {
                return (None, index);
            }
            index += 1;
        }
        let mut private = private;
        while index < path.len() {
            match self.lookup(&current, &path[index], private, depth) {
                None => return (None, index),
                Some(Target::Module(next)) => {
                    current = next;
                    index += 1;
                    private = false;
                }
                Some(other) => return (Some(other), index + 1),
            }
        }
        (Some(Target::Module(current)), index)
    }

    /// What `name` stands for in `module`: a child module, an item defined
    /// there, a `use` of that name, or one name of a glob `use`. Only
    /// re-exports count unless `private`.
    fn lookup(&self, module: &[String], name: &str, private: bool, depth: usize) -> Option<Target> {
        if depth > MAX_DEPTH {
            return None;
        }
        let mut child = module.to_vec();
        child.push(name.to_owned());
        if self.modules.contains(&child) {
            return Some(Target::Module(child));
        }
        if let Some(index) = self
            .types
            .iter()
            .position(|ty| ty.module == module && ty.def.name == name)
        {
            return Some(Target::Type(index));
        }
        if let Some(index) = self
            .functions
            .iter()
            .position(|f| f.module == module && f.function.name == name)
        {
            return Some(Target::Function(index));
        }
        let visible = |import: &&Import| import.module == module && (private || import.public);
        for import in self
            .imports
            .iter()
            .filter(visible)
            .filter(|import| import.name.as_deref() == Some(name))
        {
            let (target, consumed) =
                self.resolve_from(&import.module, &import.path, true, depth + 1);
            if consumed == import.path.len()
                && let Some(target) = target
            {
                return Some(target);
            }
        }
        let mut found: Vec<Target> = Vec::new();
        for import in self
            .imports
            .iter()
            .filter(visible)
            .filter(|import| import.name.is_none())
        {
            let (target, consumed) =
                self.resolve_from(&import.module, &import.path, true, depth + 1);
            if consumed == import.path.len()
                && let Some(Target::Module(source)) = target
                && let Some(target) = self.lookup(&source, name, false, depth + 1)
                && !found.contains(&target)
            {
                found.push(target);
            }
        }
        match found.len() {
            0 => None,
            1 => found.pop(),
            _ => Some(Target::Ambiguous(format!(
                "`{name}` comes from more than one glob `use` in {}",
                module_text(&self.name, module)
            ))),
        }
    }
}

/// `crate::a::b` for messages.
pub fn module_text(crate_name: &str, module: &[String]) -> String {
    std::iter::once(crate_name.to_owned())
        .chain(module.iter().cloned())
        .collect::<Vec<_>>()
        .join("::")
}

impl Reader<'_> {
    fn finish(mut self) -> CrateModel {
        for implementation in std::mem::take(&mut self.implementations) {
            let (target, consumed) =
                self.model
                    .resolve_from(&implementation.module, &implementation.self_path, true, 0);
            // A type of another crate, or one the reader cannot follow:
            // its methods are not this crate's types'.
            let (Some(Target::Type(index)), true) =
                (target, consumed == implementation.self_path.len())
            else {
                continue;
            };
            let def = &mut self.model.types[index].def;
            if let Some(trait_name) = &implementation.trait_name
                && !def.implements.contains(trait_name)
            {
                def.implements.push(trait_name.clone());
            }
            def.methods.extend(implementation.methods);
        }
        self.model
    }

    /// The label of a file in locations: relative to the repository root.
    fn label(&self, path: &Path) -> String {
        path.strip_prefix(self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    fn file(&mut self, path: &Path, module: &[String], children: &Path) -> Result<(), String> {
        let label = self.label(path);
        let source =
            std::fs::read_to_string(path).map_err(|err| format!("cannot read {label}: {err}"))?;
        let file = syn::parse_file(&source).map_err(|err| {
            let at = err.span().start();
            format!("{label}:{}: {err}", at.line)
        })?;
        self.items(&file.items, module, &label, Some(children))
    }

    fn items(
        &mut self,
        items: &[syn::Item],
        module: &[String],
        label: &str,
        children: Option<&Path>,
    ) -> Result<(), String> {
        let location = |span: proc_macro2::Span| Location {
            file: label.to_owned(),
            line: span.start().line,
        };
        for item in items {
            match item {
                syn::Item::Struct(item) => {
                    let mut def = TypeDef::new(
                        &item.ident.to_string(),
                        Kind::Struct,
                        item.ident.span().start().line,
                    );
                    def.generics = generics(&item.generics);
                    def.location = Some(location(item.ident.span()));
                    def.stamp = Some(stamp(item));
                    match &item.fields {
                        syn::Fields::Named(named) => {
                            def.fields = named.named.iter().map(field).collect();
                        }
                        syn::Fields::Unnamed(unnamed) => {
                            def.tuple = true;
                            def.fields = unnamed
                                .unnamed
                                .iter()
                                .enumerate()
                                .map(|(index, f)| Field {
                                    name: index.to_string(),
                                    vis: vis(&f.vis),
                                    ty: Ty::from_syn(&f.ty),
                                    line: f.ty.span().start().line,
                                })
                                .collect();
                        }
                        syn::Fields::Unit => {}
                    }
                    self.push_type(module, def);
                }
                syn::Item::Enum(item) => {
                    let mut def = TypeDef::new(
                        &item.ident.to_string(),
                        Kind::Enum,
                        item.ident.span().start().line,
                    );
                    def.generics = generics(&item.generics);
                    def.location = Some(location(item.ident.span()));
                    def.stamp = Some(stamp(item));
                    def.variants = item
                        .variants
                        .iter()
                        .map(|variant| Variant {
                            name: variant.ident.to_string(),
                            payload: payload(&variant.fields),
                            line: variant.ident.span().start().line,
                        })
                        .collect();
                    self.push_type(module, def);
                }
                syn::Item::Trait(item) => {
                    let mut def = TypeDef::new(
                        &item.ident.to_string(),
                        Kind::Trait,
                        item.ident.span().start().line,
                    );
                    def.generics = generics(&item.generics);
                    def.location = Some(location(item.ident.span()));
                    def.stamp = Some(stamp(item));
                    for trait_item in &item.items {
                        if let syn::TraitItem::Fn(function) = trait_item {
                            let mut method = method(
                                &function.sig,
                                Vis::Pub,
                                location(function.sig.ident.span()),
                            );
                            method.is_abstract = function.default.is_none();
                            method.stamp = Some(stamp(function));
                            def.methods.push(method);
                        }
                    }
                    self.push_type(module, def);
                }
                syn::Item::Fn(item) => {
                    let mut function =
                        method(&item.sig, vis(&item.vis), location(item.sig.ident.span()));
                    function.stamp = Some(stamp(item));
                    self.model.functions.push(CodeFn {
                        module: module.to_vec(),
                        function,
                    });
                }
                syn::Item::Use(item) => {
                    if item.leading_colon.is_some() {
                        continue;
                    }
                    let public = !matches!(item.vis, syn::Visibility::Inherited);
                    let mut found = Vec::new();
                    imports_of(&item.tree, &mut Vec::new(), &mut found);
                    for (name, path) in found {
                        self.model.imports.push(Import {
                            module: module.to_vec(),
                            name,
                            path,
                            public,
                        });
                    }
                }
                syn::Item::Impl(item) => {
                    let Some(self_path) = type_path(&item.self_ty) else {
                        continue;
                    };
                    let trait_name = item
                        .trait_
                        .as_ref()
                        .and_then(|(path, _)| path.segments.last())
                        .map(|segment| segment.ident.to_string());
                    let mut methods = Vec::new();
                    for impl_item in &item.items {
                        if let syn::ImplItem::Fn(function) = impl_item {
                            let visibility = if trait_name.is_some() {
                                Vis::Pub
                            } else {
                                vis(&function.vis)
                            };
                            let mut method = method(
                                &function.sig,
                                visibility,
                                location(function.sig.ident.span()),
                            );
                            method.from_trait = trait_name.clone();
                            method.stamp = Some(stamp(function));
                            methods.push(method);
                        }
                    }
                    self.implementations.push(Implementation {
                        module: module.to_vec(),
                        self_path,
                        trait_name,
                        methods,
                    });
                }
                syn::Item::Mod(item) => {
                    if is_cfg_test(&item.attrs) {
                        continue;
                    }
                    if item.attrs.iter().any(|attr| attr.path().is_ident("path")) {
                        return Err(format!(
                            "{label}:{}: `#[path]` on a module is outside what this reader follows",
                            item.ident.span().start().line
                        ));
                    }
                    let name = item.ident.to_string();
                    let mut child = module.to_vec();
                    child.push(name.clone());
                    self.model.modules.push(child.clone());
                    let child_dir = children.map(|dir| dir.join(&name));
                    match &item.content {
                        Some((_, items)) => {
                            self.items(items, &child, label, child_dir.as_deref())?
                        }
                        None => {
                            let Some(dir) = children else {
                                return Err(format!(
                                    "{label}: `mod {name};` names a file, and this source has none"
                                ));
                            };
                            let flat = dir.join(format!("{name}.rs"));
                            let nested = dir.join(&name).join("mod.rs");
                            let path: PathBuf = if flat.is_file() {
                                flat
                            } else if nested.is_file() {
                                nested
                            } else {
                                return Err(format!(
                                    "{label}: `mod {name};` names neither {} nor {}",
                                    self.label(&flat),
                                    self.label(&nested)
                                ));
                            };
                            self.file(&path, &child, &dir.join(&name))?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn push_type(&mut self, module: &[String], def: TypeDef) {
        self.model.types.push(CodeType {
            module: module.to_vec(),
            def,
        });
    }
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string().trim() == "test")
    })
}

fn generics(generics: &syn::Generics) -> Vec<String> {
    generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(ty) => Some(ty.ident.to_string()),
            _ => None,
        })
        .collect()
}

fn vis(vis: &syn::Visibility) -> Vis {
    match vis {
        syn::Visibility::Public(_) => Vis::Pub,
        syn::Visibility::Restricted(restricted) => {
            if restricted.in_token.is_none() && restricted.path.is_ident("self") {
                Vis::Private
            } else {
                Vis::Crate
            }
        }
        syn::Visibility::Inherited => Vis::Private,
    }
}

fn field(field: &syn::Field) -> Field {
    Field {
        name: field
            .ident
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        vis: vis(&field.vis),
        ty: Ty::from_syn(&field.ty),
        // The name's line: the field's span starts at its doc comment.
        line: field
            .ident
            .as_ref()
            .map_or_else(|| field.ty.span(), proc_macro2::Ident::span)
            .start()
            .line,
    }
}

fn payload(fields: &syn::Fields) -> Payload {
    match fields {
        syn::Fields::Unit => Payload::Unit,
        syn::Fields::Unnamed(unnamed) => Payload::Tuple(
            unnamed
                .unnamed
                .iter()
                .map(|f| Ty::from_syn(&f.ty))
                .collect(),
        ),
        syn::Fields::Named(named) => Payload::Named(
            named
                .named
                .iter()
                .map(|f| {
                    (
                        f.ident
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_default(),
                        Ty::from_syn(&f.ty),
                    )
                })
                .collect(),
        ),
    }
}

fn method(sig: &syn::Signature, visibility: Vis, location: Location) -> Method {
    let receiver = match sig.receiver() {
        None => Receiver::None,
        Some(receiver) => match &receiver.kind {
            syn::ReceiverKind::Value => Receiver::Value,
            syn::ReceiverKind::Reference(_, _, mutability) => {
                if mutability.is_some() {
                    Receiver::RefMut
                } else {
                    Receiver::Ref
                }
            }
            syn::ReceiverKind::Typed(_, ty) => Receiver::Typed(Ty::from_syn(ty).canon),
            _ => Receiver::Value,
        },
    };
    let params = sig
        .inputs
        .iter()
        .filter_map(|input| match input {
            syn::FnArg::Typed(typed) => {
                let name = match &*typed.pat {
                    syn::Pat::Ident(ident) => Some(ident.ident.to_string()),
                    other => Some(other.to_token_stream().to_string()),
                };
                let ty = Some(typed.ty.to_token_stream().to_string());
                Some(Param {
                    text: format!(
                        "{}: {}",
                        name.clone().unwrap_or_default(),
                        ty.clone().unwrap_or_default()
                    ),
                    name,
                    ty,
                })
            }
            _ => None,
        })
        .collect();
    let ret = match &sig.output {
        syn::ReturnType::Default => None,
        syn::ReturnType::Type(_, ty) => Some(ty.to_token_stream().to_string()),
    };
    Method {
        name: sig.ident.to_string(),
        vis: visibility,
        receiver: Some(receiver),
        is_abstract: false,
        params,
        ret,
        from_trait: None,
        line: location.line,
        location: Some(location),
        stamp: None,
    }
}

/// The path of a type, without its generic arguments: `a::X<T>` is
/// `["a", "X"]`; `None` for what is not a path.
fn type_path(ty: &syn::Type) -> Option<Vec<String>> {
    match ty {
        syn::Type::Path(path) if path.qself.is_none() && path.path.leading_colon.is_none() => Some(
            path.path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect(),
        ),
        syn::Type::Reference(reference) => type_path(&reference.elem),
        syn::Type::Paren(paren) => type_path(&paren.elem),
        syn::Type::Group(group) => type_path(&group.elem),
        _ => None,
    }
}

/// The names a `use` tree brings in, with their paths; `None` for a glob.
fn imports_of(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    out: &mut Vec<(Option<String>, Vec<String>)>,
) {
    let with = |prefix: &[String], last: &syn::Ident| {
        let mut path = prefix.to_vec();
        if last != "self" {
            path.push(last.to_string());
        }
        path
    };
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            imports_of(&path.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let path = with(prefix, &name.ident);
            if let Some(last) = path.last() {
                out.push((Some(last.clone()), path));
            }
        }
        syn::UseTree::Rename(rename) => {
            if rename.rename != "_" {
                out.push((Some(rename.rename.to_string()), with(prefix, &rename.ident)));
            }
        }
        syn::UseTree::Glob(_) => out.push((None, prefix.clone())),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                imports_of(item, prefix, out);
            }
        }
    }
}

/// An item's stamp: the first 8 hex digits of the SHA-256 of its tokens
/// without doc comments.
pub fn stamp(item: &impl ToTokens) -> String {
    let tokens = without_docs(item.to_token_stream()).to_string();
    let mut hash = helpers::hash::sha256_bytes(tokens.as_bytes());
    hash.truncate(8);
    hash
}

/// The stream without `#[doc = …]` and `#![doc = …]` attributes, at any
/// depth.
fn without_docs(stream: TokenStream) -> TokenStream {
    let trees: Vec<TokenTree> = stream.into_iter().collect();
    let mut out = Vec::with_capacity(trees.len());
    let mut index = 0;
    while index < trees.len() {
        if let TokenTree::Punct(pound) = &trees[index]
            && pound.as_char() == '#'
        {
            let bang =
                matches!(&trees.get(index + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!');
            let group_at = index + 1 + usize::from(bang);
            if let Some(TokenTree::Group(group)) = trees.get(group_at)
                && group.delimiter() == Delimiter::Bracket
                && matches!(group.stream().into_iter().next(), Some(TokenTree::Ident(ident)) if ident == "doc")
            {
                index = group_at + 1;
                continue;
            }
        }
        match &trees[index] {
            TokenTree::Group(group) => {
                let mut inner = Group::new(group.delimiter(), without_docs(group.stream()));
                inner.set_span(group.span());
                out.push(TokenTree::Group(inner));
            }
            other => out.push(other.clone()),
        }
        index += 1;
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "
/// A shape.
pub struct Circle {
    /// Its radius.
    pub r: f64,
    name: String,
}

impl Circle {
    /// Its area.
    pub fn area(&self) -> f64 { 3.0 * self.r * self.r }
    fn rename(&mut self, name: String) { self.name = name; }
}

impl std::fmt::Display for Circle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }
}

pub enum Outcome { Pass, Fail(String), Ended { how: String } }

pub trait Store { fn get(&self) -> u8; fn put(&mut self) {} }

pub(crate) fn helper(x: u8) -> u8 { x }

#[cfg(test)]
mod tests { pub struct Circle; }
";

    #[test]
    fn items_read_with_their_methods() {
        let model = CrateModel::from_source("demo", "demo.rs", SOURCE).unwrap();
        assert_eq!(model.types.len(), 3, "the test module is skipped");
        let circle = &model.types_named("Circle")[0].def;
        assert_eq!(circle.fields[1].vis, Vis::Private);
        assert_eq!(circle.location.as_ref().unwrap().line, 3);
        let names: Vec<&str> = circle.methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["area", "rename", "fmt"]);
        assert_eq!(circle.methods[1].receiver, Some(Receiver::RefMut));
        assert_eq!(circle.methods[2].vis, Vis::Pub);
        assert_eq!(circle.implements, ["Display"]);
        let outcome = &model.types_named("Outcome")[0].def;
        assert!(matches!(outcome.variants[2].payload, Payload::Named(_)));
        let store = &model.types_named("Store")[0].def;
        assert!(store.methods[0].is_abstract && !store.methods[1].is_abstract);
        assert_eq!(model.functions_named("helper")[0].function.vis, Vis::Crate);
    }

    #[test]
    fn stamps_ignore_docs_and_formatting_but_not_code() {
        let a = CrateModel::from_source("a", "a.rs", "/// one\nfn f() -> u8 { 1 }").unwrap();
        let b =
            CrateModel::from_source("b", "b.rs", "/// two\n// a comment\nfn f()->u8{\n    1\n}")
                .unwrap();
        let c = CrateModel::from_source("c", "c.rs", "fn f() -> u8 { 2 }").unwrap();
        let stamp = |m: &CrateModel| m.functions[0].function.stamp.clone().unwrap();
        assert_eq!(stamp(&a), stamp(&b));
        assert_ne!(stamp(&a), stamp(&c));
    }

    #[test]
    fn paths_resolve_through_modules_and_reexports() {
        let source =
            "pub mod a { pub mod b { pub struct T; impl T { pub fn m(&self) {} } pub fn f() {} } }
                      pub use a::b::T;";
        let model = CrateModel::from_source("demo", "demo.rs", source).unwrap();
        assert!(matches!(model.resolve(&["a", "b"]), Resolved::Module(_)));
        assert!(matches!(
            model.resolve(&["a", "b", "f"]),
            Resolved::Function(_)
        ));
        assert!(matches!(
            model.resolve(&["a", "b", "T", "m"]),
            Resolved::Method(..)
        ));
        assert!(
            matches!(model.resolve(&["T"]), Resolved::Type(_)),
            "the `pub use` at the root"
        );
        assert!(matches!(model.resolve(&["T", "m"]), Resolved::Method(..)));
        assert!(matches!(model.resolve(&["a", "x"]), Resolved::Nothing));
        assert!(
            matches!(model.resolve(&["f"]), Resolved::Nothing),
            "no `use` brings `f` to the root"
        );
    }

    #[test]
    fn a_name_no_module_defines_or_re_exports_names_nothing() {
        let model = CrateModel::from_source(
            "demo",
            "demo.rs",
            "pub mod a { pub struct X; } pub mod b {} use a::X as Hidden;",
        )
        .unwrap();
        assert!(matches!(model.resolve(&["a", "X"]), Resolved::Type(_)));
        for path in [&["b", "X"][..], &["X"], &["Hidden"]] {
            assert!(
                matches!(model.resolve(path), Resolved::Nothing),
                "{path:?} names nothing: a same-named item elsewhere, or a private `use`, proves no path"
            );
        }
    }

    #[test]
    fn re_exports_resolve_named_renamed_and_by_glob() {
        let source = "
            pub mod a { pub struct X; impl X { pub fn m(&self) {} } pub fn f() {} }
            pub mod b { pub use super::a::X; pub use crate::a::{self as inner, f}; }
            pub mod c { pub use crate::a::*; }
            pub mod d { pub mod e { pub use super::super::a::X as Y; } }
            pub use a::X as Z;
        ";
        let model = CrateModel::from_source("demo", "demo.rs", source).unwrap();
        let ty = |path: &[&str]| match model.resolve(path) {
            Resolved::Type(ty) => ty.module.clone(),
            other => panic!("{path:?}: {other:?}"),
        };
        let a = vec!["a".to_owned()];
        assert_eq!(ty(&["b", "X"]), a);
        assert_eq!(ty(&["c", "X"]), a);
        assert_eq!(ty(&["d", "e", "Y"]), a);
        assert_eq!(ty(&["Z"]), a);
        assert_eq!(ty(&["b", "inner", "X"]), a);
        assert!(matches!(model.resolve(&["Z", "m"]), Resolved::Method(..)));
        assert!(matches!(model.resolve(&["b", "f"]), Resolved::Function(_)));
        assert!(matches!(model.resolve(&["c", "f"]), Resolved::Function(_)));
        assert!(matches!(model.resolve(&["c", "Y"]), Resolved::Nothing));
    }

    #[test]
    fn glob_conflicts_and_cycles_are_not_guessed() {
        let source = "
            pub mod a { pub struct X; }
            pub mod b { pub struct X; }
            pub mod c { pub use super::a::*; pub use super::b::*; }
            pub mod p { pub use super::q::L; }
            pub mod q { pub use super::p::L; }
        ";
        let model = CrateModel::from_source("demo", "demo.rs", source).unwrap();
        assert!(matches!(model.resolve(&["c", "X"]), Resolved::Ambiguous(_)));
        assert!(matches!(model.resolve(&["p", "L"]), Resolved::Nothing));
    }

    #[test]
    fn an_impl_goes_to_the_type_its_path_names() {
        let source = "
            pub mod a { pub struct X; }
            pub mod b { pub struct X; }
            impl a::X { pub fn foo(&self) {} }
            impl crate::b::X { pub fn bar(&self) {} }
            use b::X as Bx;
            impl Bx { pub fn baz(&self) {} }
            mod c { impl super::a::X { pub fn qux(&self) {} } }
            impl std::fmt::Display for a::X {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }
            }
            impl Vec<u8> { fn not_ours(&self) {} }
        ";
        let model = CrateModel::from_source("demo", "demo.rs", source).unwrap();
        let methods = |path: &[&str]| match model.resolve(path) {
            Resolved::Type(ty) => ty
                .def
                .methods
                .iter()
                .map(|m| m.name.clone())
                .collect::<Vec<_>>(),
            other => panic!("{path:?}: {other:?}"),
        };
        assert_eq!(methods(&["a", "X"]), ["foo", "qux", "fmt"]);
        assert_eq!(methods(&["b", "X"]), ["bar", "baz"]);
        assert!(matches!(
            model.resolve(&["a", "X", "foo"]),
            Resolved::Method(..)
        ));
        assert!(matches!(
            model.resolve(&["b", "X", "foo"]),
            Resolved::Nothing
        ));
        match model.resolve(&["a", "X"]) {
            Resolved::Type(ty) => assert_eq!(ty.def.implements, ["Display"]),
            other => panic!("{other:?}"),
        }
    }
}
