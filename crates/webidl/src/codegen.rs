//! Rust binding stubs from merged IDL interfaces.
//!
//! For each interface the generator emits:
//! - a trait named after the interface, with the parent interface's trait as
//!   its supertrait, and one method per attribute getter, attribute setter,
//!   operation overload, constructor overload and anonymous special
//!   operation. Every method takes the receiver as an [`ObjectId`]
//!   (`this`), never a pointer, and has a default body returning
//!   `BindingError::NotImplemented`, so the host can fill members in one by one;
//! - a `<snake_name>_consts` module with the interface's constants;
//! - an entry in `INTERFACES` describing every member, for registering the
//!   interface with the engine.
//!
//! The output depends only on the [`IdlSet`] (which is order-independent),
//! the interface list and the header note: no maps with random iteration
//! order, no timestamps, no paths.
//!
//! [`ObjectId`]: crate::runtime::ObjectId

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::IdlSet;
use crate::ast::*;
use crate::model::{MergedInterface, MergedMember, NameKind};

/// The interfaces [`crate::stubs`] is generated for, parents first.
pub const STUB_INTERFACES: &[&str] =
    &["EventTarget", "Node", "Element", "Document", "DOMTokenList"];

/// Typedefs and nested types deeper than this are an error (a typedef cycle).
const MAX_TYPE_DEPTH: u32 = 32;

/// An error generating stubs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GenError {
    /// No non-partial interface of that name.
    #[error("no interface `{0}` in the IDL set")]
    UnknownInterface(String),
    /// An interface's parent is not in the list being generated.
    #[error("`{interface}` inherits `{parent}`, which is not being generated")]
    ParentNotGenerated {
        /// The interface.
        interface: String,
        /// Its parent.
        parent: String,
    },
    /// Two members map to the same Rust name.
    #[error("`{interface}`: two members map to the Rust name `{name}`")]
    NameClash {
        /// The interface.
        interface: String,
        /// The clashing Rust name.
        name: String,
    },
    /// A type nests or aliases too deeply (a typedef cycle).
    #[error("type `{0}` nests too deeply (typedef cycle?)")]
    TooDeep(String),
    /// A constant whose value doesn't fit its type or can't be read.
    #[error("`{interface}.{name}`: bad constant value `{value}`")]
    BadConst {
        /// The interface.
        interface: String,
        /// The constant.
        name: String,
        /// The value as written.
        value: String,
    },
    /// More than 255 overloads.
    #[error("`{interface}.{name}`: too many overloads")]
    TooManyOverloads {
        /// The interface.
        interface: String,
        /// The operation.
        name: String,
    },
    /// The interface is listed twice.
    #[error("`{0}` listed twice")]
    Duplicate(String),
    /// The interface's parent chain leads back to an interface already on it
    /// (`interface J : J`, or `A : B` and `B : A`).
    #[error("`{interface}` has an inheritance cycle: {}", chain.join(" : "))]
    InheritanceCycle {
        /// The interface being generated.
        interface: String,
        /// The parent chain from `interface`, ending with the repeated name.
        chain: Vec<String>,
    },
}

/// Walks `name`'s parent chain and fails if a name comes back. Iterative and
/// bounded: every step adds a distinct interface name, and the set has
/// finitely many.
fn check_inheritance(set: &IdlSet, name: &str) -> Result<(), GenError> {
    let mut chain = vec![name.to_owned()];
    let mut seen = BTreeSet::from([name.to_owned()]);
    let mut current = set.interface(name).and_then(|m| m.parent);
    while let Some(parent) = current {
        chain.push(parent.clone());
        if !seen.insert(parent.clone()) {
            return Err(GenError::InheritanceCycle {
                interface: name.to_owned(),
                chain,
            });
        }
        current = set.interface(&parent).and_then(|m| m.parent);
    }
    Ok(())
}

/// The Rust identifier for an IDL name: `camelCase` to `snake_case`
/// (`innerHTML` → `inner_html`, `getElementsByTagNameNS` →
/// `get_elements_by_tag_name_ns`), with `_` appended to Rust keywords.
pub fn rust_name(idl: &str) -> String {
    let chars: Vec<char> = idl.chars().collect();
    let mut out = String::with_capacity(idl.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c == '-' {
            out.push('_');
        } else if c.is_ascii_uppercase() {
            let prev = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
            let next = chars.get(i + 1).copied();
            let boundary = match prev {
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_ascii_uppercase() => next.is_some_and(|n| n.is_ascii_lowercase()),
                _ => false,
            };
            if boundary && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    const KEYWORDS: &[&str] = &[
        "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
        "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
        "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub",
        "ref", "return", "self", "static", "struct", "super", "trait", "true", "try", "type",
        "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield", "this",
    ];
    if KEYWORDS.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

/// Generates the stubs for `interfaces`, in that order. `header` is written
/// as a comment at the top (say where the IDL came from).
pub fn generate_stubs(set: &IdlSet, interfaces: &[&str], header: &str) -> Result<String, GenError> {
    let mut g = Gen {
        set,
        names: set.names(),
        used_value: false,
    };
    let mut merged = Vec::new();
    for (i, name) in interfaces.iter().enumerate() {
        if interfaces[..i].contains(name) {
            return Err(GenError::Duplicate((*name).to_owned()));
        }
        let m = set
            .interface(name)
            .ok_or_else(|| GenError::UnknownInterface((*name).to_owned()))?;
        check_inheritance(set, name)?;
        if let Some(p) = &m.parent {
            if !interfaces.contains(&p.as_str()) {
                return Err(GenError::ParentNotGenerated {
                    interface: m.name.clone(),
                    parent: p.clone(),
                });
            }
        }
        merged.push(m);
    }

    let mut body = String::new();
    let mut infos = String::new();
    for m in &merged {
        let (code, info) = g.interface(m)?;
        body.push_str(&code);
        infos.push_str(&info);
    }

    let mut out = String::new();
    for line in header.lines() {
        let _ = writeln!(out, "// {line}");
    }
    out.push_str("// @generated by surimono-webidl. Do not edit by hand; regenerate with\n");
    out.push_str("// `cargo run -p surimono-webidl --example regen`.\n\n");
    out.push_str("#![allow(clippy::too_many_arguments, clippy::type_complexity)]\n\n");
    out.push_str("use crate::runtime::{BindingError, BindingResult, InterfaceInfo, MemberInfo, MemberInfoKind, ObjectId");
    if g.used_value {
        out.push_str(", Value");
    }
    out.push_str("};\n\n");
    out.push_str("/// Every generated interface, in generation order.\n");
    out.push_str("pub static INTERFACES: &[InterfaceInfo] = &[\n");
    out.push_str(&infos);
    out.push_str("];\n");
    out.push_str(&body);
    Ok(out)
}

struct Gen<'a> {
    set: &'a IdlSet,
    names: std::collections::BTreeMap<String, NameKind>,
    used_value: bool,
}

/// A Rust method to emit.
struct Method {
    rust: String,
    doc: Vec<String>,
    params: Vec<(String, String)>,
    has_this: bool,
    ret: String,
    js_name: String,
}

fn quote(s: &str) -> String {
    format!("{s:?}")
}

fn source_note(m: &MergedMember, iface: &str) -> String {
    if m.declared_in == iface {
        m.file.clone()
    } else {
        format!("{}, {}", m.declared_in, m.file)
    }
}

impl Gen<'_> {
    fn value(&mut self) -> String {
        self.used_value = true;
        "Value".to_owned()
    }

    fn rust_type(&mut self, t: &Type, depth: u32) -> Result<String, GenError> {
        if depth > MAX_TYPE_DEPTH {
            return Err(GenError::TooDeep(t.to_string()));
        }
        let (inner, nullable) = match &t.kind {
            TypeKind::Named(n) => match n.as_str() {
                "undefined" => ("()".to_owned(), false),
                "boolean" => ("bool".to_owned(), t.nullable),
                "byte" => ("i8".to_owned(), t.nullable),
                "octet" => ("u8".to_owned(), t.nullable),
                "short" => ("i16".to_owned(), t.nullable),
                "unsigned short" => ("u16".to_owned(), t.nullable),
                "long" => ("i32".to_owned(), t.nullable),
                "unsigned long" => ("u32".to_owned(), t.nullable),
                "long long" => ("i64".to_owned(), t.nullable),
                "unsigned long long" => ("u64".to_owned(), t.nullable),
                "float" | "unrestricted float" => ("f32".to_owned(), t.nullable),
                "double" | "unrestricted double" => ("f64".to_owned(), t.nullable),
                "DOMString" | "USVString" | "ByteString" | "CSSOMString" => {
                    ("String".to_owned(), t.nullable)
                }
                other => match self.names.get(other) {
                    Some(NameKind::Interface) => ("ObjectId".to_owned(), t.nullable),
                    Some(NameKind::Enum) => ("String".to_owned(), t.nullable),
                    Some(NameKind::Typedef) => {
                        let Some(target) = self.set.typedef(other).cloned() else {
                            return Ok(self.value());
                        };
                        let resolved = self.rust_type(
                            &Type {
                                nullable: target.nullable || t.nullable,
                                ..target
                            },
                            depth + 1,
                        )?;
                        return Ok(resolved);
                    }
                    // any, object, symbol, bigint, buffer sources,
                    // dictionaries, callbacks, callback interfaces and
                    // names outside the vendored IDL.
                    _ => (self.value(), false),
                },
            },
            TypeKind::Sequence(e) | TypeKind::FrozenArray(e) | TypeKind::ObservableArray(e) => (
                format!("Vec<{}>", self.rust_type(e, depth + 1)?),
                t.nullable,
            ),
            TypeKind::Record(k, v) => (
                format!(
                    "Vec<({}, {})>",
                    self.rust_type(k, depth + 1)?,
                    self.rust_type(v, depth + 1)?
                ),
                t.nullable,
            ),
            TypeKind::Promise(_) | TypeKind::Union(_) => (self.value(), false),
        };
        Ok(if nullable && inner != "Value" {
            format!("Option<{inner}>")
        } else {
            inner
        })
    }

    fn params(&mut self, args: &[Argument]) -> Result<Vec<(String, String)>, GenError> {
        let mut out = Vec::new();
        for a in args {
            let t = self.rust_type(&a.ty, 0)?;
            let t = if a.variadic {
                format!("Vec<{t}>")
            } else if a.optional && a.default.is_none() {
                format!("Option<{t}>")
            } else {
                t
            };
            out.push((rust_name(&a.name), t));
        }
        Ok(out)
    }

    fn const_value(
        &self,
        iface: &str,
        name: &str,
        ty: &str,
        v: &ConstValue,
    ) -> Result<String, GenError> {
        let bad = || GenError::BadConst {
            interface: iface.to_owned(),
            name: name.to_owned(),
            value: v.to_string(),
        };
        Ok(match v {
            ConstValue::Bool(b) => b.to_string(),
            ConstValue::Integer(s) => {
                let (neg, digits) = match s.strip_prefix('-') {
                    Some(d) => (true, d),
                    None => (false, s.as_str()),
                };
                let n = if let Some(h) = digits
                    .strip_prefix("0x")
                    .or_else(|| digits.strip_prefix("0X"))
                {
                    i128::from_str_radix(h, 16)
                } else if digits.len() > 1 && digits.starts_with('0') {
                    i128::from_str_radix(&digits[1..], 8)
                } else {
                    digits.parse::<i128>()
                }
                .map_err(|_| bad())?;
                let n = if neg { -n } else { n };
                let fits = match ty {
                    "i8" => i8::try_from(n).is_ok(),
                    "u8" => u8::try_from(n).is_ok(),
                    "i16" => i16::try_from(n).is_ok(),
                    "u16" => u16::try_from(n).is_ok(),
                    "i32" => i32::try_from(n).is_ok(),
                    "u32" => u32::try_from(n).is_ok(),
                    "i64" => i64::try_from(n).is_ok(),
                    "u64" => u64::try_from(n).is_ok(),
                    "f32" | "f64" => true,
                    _ => false,
                };
                if !fits {
                    return Err(bad());
                }
                if ty.starts_with('f') {
                    format!("{n}.0")
                } else {
                    n.to_string()
                }
            }
            ConstValue::Float(s) => {
                if !ty.starts_with('f') {
                    return Err(bad());
                }
                match s.as_str() {
                    "Infinity" => format!("{ty}::INFINITY"),
                    "-Infinity" => format!("{ty}::NEG_INFINITY"),
                    "NaN" => format!("{ty}::NAN"),
                    _ => format!("{:?}", s.parse::<f64>().map_err(|_| bad())?),
                }
            }
        })
    }

    /// Returns (trait + consts code, INTERFACES entry).
    fn interface(&mut self, m: &MergedInterface) -> Result<(String, String), GenError> {
        let iface = m.name.as_str();
        let mut methods: Vec<Method> = Vec::new();
        let mut consts: Vec<String> = Vec::new();
        let mut infos: Vec<String> = Vec::new();
        let mut collections: Vec<String> = Vec::new();

        // Operations are grouped by (name, static): every overload of a name,
        // wherever declared, gets numbered in merge order.
        let mut done_ops: BTreeSet<(String, bool)> = BTreeSet::new();
        let mut constructors_done = false;

        for mm in &m.members {
            let idl = mm.member.to_string();
            let note = source_note(mm, iface);
            let doc = vec![format!("`{idl}` ({note})")];
            match &mm.member.kind {
                MemberKind::Const { ty, name, value } => {
                    let rt = self.rust_type(ty, 0)?;
                    let v = self.const_value(iface, name, &rt, value)?;
                    consts.push(format!(
                        "    /// `{idl}` ({note})\n    pub const {name}: {rt} = {v};\n"
                    ));
                    infos.push(format!(
                        "MemberInfo {{ idl_name: {}, rust_name: {}, kind: MemberInfoKind::Const }}",
                        quote(name),
                        quote(name)
                    ));
                }
                MemberKind::Attribute {
                    name,
                    ty,
                    readonly,
                    is_static,
                    ..
                } => {
                    let rt = self.rust_type(ty, 0)?;
                    let rust = rust_name(name);
                    methods.push(Method {
                        rust: rust.clone(),
                        doc: doc.clone(),
                        params: vec![],
                        has_this: !is_static,
                        ret: rt.clone(),
                        js_name: format!("{iface}.{name}"),
                    });
                    if !readonly {
                        let setter = format!("set_{}", rust.trim_end_matches('_'));
                        methods.push(Method {
                            rust: setter,
                            doc,
                            params: vec![("value".to_owned(), rt)],
                            has_this: !is_static,
                            ret: "()".to_owned(),
                            js_name: format!("{iface}.{name}"),
                        });
                    }
                    infos.push(format!(
                        "MemberInfo {{ idl_name: {}, rust_name: {}, kind: MemberInfoKind::Attribute {{ readonly: {readonly}, is_static: {is_static} }} }}",
                        quote(name),
                        quote(&rust)
                    ));
                }
                MemberKind::Operation {
                    name: Some(name),
                    is_static,
                    ..
                } => {
                    if !done_ops.insert((name.clone(), *is_static)) {
                        continue;
                    }
                    let base = rust_name(name);
                    let mut count = 0u32;
                    for other in &m.members {
                        let MemberKind::Operation {
                            name: Some(n2),
                            is_static: s2,
                            return_type,
                            args,
                            ..
                        } = &other.member.kind
                        else {
                            continue;
                        };
                        if n2 != name || s2 != is_static {
                            continue;
                        }
                        count += 1;
                        let rust = if count == 1 {
                            base.clone()
                        } else {
                            format!("{}_{count}", base.trim_end_matches('_'))
                        };
                        let params = self.params(args)?;
                        let ret = self.rust_type(return_type, 0)?;
                        methods.push(Method {
                            rust,
                            doc: vec![format!(
                                "`{}` ({})",
                                other.member,
                                source_note(other, iface)
                            )],
                            params,
                            has_this: !is_static,
                            ret,
                            js_name: format!("{iface}.{name}"),
                        });
                    }
                    let overloads =
                        u8::try_from(count).map_err(|_| GenError::TooManyOverloads {
                            interface: iface.to_owned(),
                            name: name.clone(),
                        })?;
                    infos.push(format!(
                        "MemberInfo {{ idl_name: {}, rust_name: {}, kind: MemberInfoKind::Operation {{ is_static: {is_static}, overloads: {overloads} }} }}",
                        quote(name),
                        quote(&base)
                    ));
                }
                MemberKind::Operation {
                    name: None,
                    special,
                    return_type,
                    args,
                    ..
                } => {
                    let indexed = args
                        .first()
                        .is_some_and(|a| a.ty.kind == TypeKind::Named("unsigned long".into()));
                    let which = match special {
                        Some(Special::Getter) => "getter",
                        Some(Special::Setter) => "setter",
                        Some(Special::Deleter) => "deleter",
                        // The parser only accepts anonymous operations with a special.
                        None => "operation",
                    };
                    let rust = format!("{}_{which}", if indexed { "indexed" } else { "named" });
                    let params = self.params(args)?;
                    let ret = self.rust_type(return_type, 0)?;
                    methods.push(Method {
                        rust: rust.clone(),
                        doc,
                        params,
                        has_this: true,
                        ret,
                        js_name: format!("{iface} {rust}"),
                    });
                    infos.push(format!(
                        "MemberInfo {{ idl_name: \"\", rust_name: {}, kind: MemberInfoKind::Special }}",
                        quote(&rust)
                    ));
                }
                MemberKind::Constructor { .. } => {
                    if constructors_done {
                        continue;
                    }
                    constructors_done = true;
                    let mut count = 0u32;
                    for other in &m.members {
                        let MemberKind::Constructor { args } = &other.member.kind else {
                            continue;
                        };
                        count += 1;
                        let rust = if count == 1 {
                            "constructor".to_owned()
                        } else {
                            format!("constructor_{count}")
                        };
                        let params = self.params(args)?;
                        methods.push(Method {
                            rust,
                            doc: vec![format!(
                                "`{}` ({})",
                                other.member,
                                source_note(other, iface)
                            )],
                            params,
                            has_this: false,
                            ret: "ObjectId".to_owned(),
                            js_name: format!("new {iface}"),
                        });
                    }
                    let overloads =
                        u8::try_from(count).map_err(|_| GenError::TooManyOverloads {
                            interface: iface.to_owned(),
                            name: "constructor".to_owned(),
                        })?;
                    infos.push(format!(
                        "MemberInfo {{ idl_name: \"constructor\", rust_name: \"constructor\", kind: MemberInfoKind::Constructor {{ overloads: {overloads} }} }}"
                    ));
                }
                MemberKind::Stringifier => {
                    methods.push(Method {
                        rust: "stringifier".to_owned(),
                        doc,
                        params: vec![],
                        has_this: true,
                        ret: "String".to_owned(),
                        js_name: format!("{iface} stringifier"),
                    });
                    infos.push(
                        "MemberInfo { idl_name: \"\", rust_name: \"stringifier\", kind: MemberInfoKind::Special }".to_owned(),
                    );
                }
                MemberKind::Iterable { .. }
                | MemberKind::Maplike { .. }
                | MemberKind::Setlike { .. } => {
                    collections.push(format!("`{idl}` ({note})"));
                    let kw = match &mm.member.kind {
                        MemberKind::Iterable { .. } => "iterable",
                        MemberKind::Maplike { .. } => "maplike",
                        _ => "setlike",
                    };
                    infos.push(format!(
                        "MemberInfo {{ idl_name: {}, rust_name: \"\", kind: MemberInfoKind::Collection }}",
                        quote(kw)
                    ));
                }
            }
        }

        let mut seen = BTreeSet::new();
        for meth in &methods {
            if !seen.insert(meth.rust.as_str()) {
                return Err(GenError::NameClash {
                    interface: iface.to_owned(),
                    name: meth.rust.clone(),
                });
            }
        }

        // The trait.
        let mut code = String::new();
        let _ = writeln!(code);
        let header = match &m.parent {
            Some(p) => format!("interface {iface} : {p}"),
            None => format!("interface {iface}"),
        };
        let _ = writeln!(code, "/// IDL `{header}` ({}).", m.file);
        if !m.mixins.is_empty() {
            let _ = writeln!(code, "///");
            let _ = writeln!(code, "/// Includes: {}.", m.mixins.join(", "));
        }
        let others: Vec<&str> = m
            .files
            .iter()
            .map(String::as_str)
            .filter(|f| *f != m.file)
            .collect();
        if !others.is_empty() {
            let _ = writeln!(code, "///");
            let _ = writeln!(code, "/// Members also from: {}.", others.join(", "));
        }
        if !collections.is_empty() {
            let _ = writeln!(code, "///");
            let _ = writeln!(code, "/// Not generated yet:");
            for c in &collections {
                let _ = writeln!(code, "/// - {c}");
            }
        }
        let supertrait = m
            .parent
            .as_ref()
            .map(|p| format!(": {p}"))
            .unwrap_or_default();
        let _ = writeln!(code, "pub trait {iface}{supertrait} {{");
        for (i, meth) in methods.iter().enumerate() {
            if i > 0 {
                code.push('\n');
            }
            for d in &meth.doc {
                let _ = writeln!(code, "    /// {d}");
            }
            let mut params = String::from("&mut self");
            let mut names = Vec::new();
            if meth.has_this {
                params.push_str(", this: ObjectId");
                names.push("this".to_owned());
            }
            for (n, t) in &meth.params {
                let _ = write!(params, ", {n}: {t}");
                names.push(n.clone());
            }
            let _ = writeln!(
                code,
                "    fn {}({params}) -> BindingResult<{}> {{",
                meth.rust, meth.ret
            );
            match names.len() {
                0 => {}
                1 => {
                    let _ = writeln!(code, "        let _ = {};", names[0]);
                }
                _ => {
                    let _ = writeln!(code, "        let _ = ({});", names.join(", "));
                }
            }
            let _ = writeln!(
                code,
                "        Err(BindingError::NotImplemented({}))",
                quote(&meth.js_name)
            );
            let _ = writeln!(code, "    }}");
        }
        let _ = writeln!(code, "}}");

        if !consts.is_empty() {
            let _ = writeln!(code);
            let _ = writeln!(code, "/// Constants of `{iface}`.");
            let _ = writeln!(
                code,
                "pub mod {}_consts {{",
                rust_name(iface).trim_end_matches('_')
            );
            for (i, c) in consts.iter().enumerate() {
                if i > 0 {
                    code.push('\n');
                }
                code.push_str(c);
            }
            let _ = writeln!(code, "}}");
        }

        let mut info = String::new();
        let _ = writeln!(info, "    InterfaceInfo {{");
        let _ = writeln!(info, "        name: {},", quote(iface));
        match &m.parent {
            Some(p) => {
                let _ = writeln!(info, "        parent: Some({}),", quote(p));
            }
            None => {
                let _ = writeln!(info, "        parent: None,");
            }
        }
        let files: Vec<String> = m.files.iter().map(|f| quote(f)).collect();
        let _ = writeln!(info, "        files: &[{}],", files.join(", "));
        let _ = writeln!(info, "        members: &[");
        for i in &infos {
            let _ = writeln!(info, "            {i},");
        }
        let _ = writeln!(info, "        ],");
        let _ = writeln!(info, "    }},");
        Ok((code, info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_names() {
        for (idl, rust) in [
            ("nodeType", "node_type"),
            ("innerHTML", "inner_html"),
            ("getElementsByTagNameNS", "get_elements_by_tag_name_ns"),
            ("URL", "url"),
            ("documentURI", "document_uri"),
            ("XMLHttpRequest", "xml_http_request"),
            ("type", "type_"),
            ("ELEMENT_NODE", "element_node"),
            ("x2y", "x2y"),
            ("aria-label", "aria_label"),
        ] {
            assert_eq!(rust_name(idl), rust, "{idl}");
        }
    }

    fn set(src: &str) -> IdlSet {
        IdlSet::parse_sources([("t.idl", src)]).unwrap()
    }

    #[test]
    fn errors() {
        let s = set("interface A : B {}; interface C { attribute long x; undefined setX(); };");
        assert_eq!(
            generate_stubs(&s, &["Z"], "").unwrap_err(),
            GenError::UnknownInterface("Z".into())
        );
        assert!(matches!(
            generate_stubs(&s, &["A"], "").unwrap_err(),
            GenError::ParentNotGenerated { .. }
        ));
        assert!(matches!(
            generate_stubs(&s, &["C"], "").unwrap_err(),
            GenError::NameClash { .. }
        ));
        assert!(matches!(
            generate_stubs(&s, &["C", "C"], "").unwrap_err(),
            GenError::Duplicate(_)
        ));
        let s = set("typedef T2 T1; typedef T1 T2; interface A { attribute T1 x; };");
        assert!(matches!(
            generate_stubs(&s, &["A"], "").unwrap_err(),
            GenError::TooDeep(_)
        ));
        let s = set("interface J : J {};");
        assert_eq!(
            generate_stubs(&s, &["J"], "").unwrap_err(),
            GenError::InheritanceCycle {
                interface: "J".into(),
                chain: vec!["J".into(), "J".into()],
            }
        );
        let s = set("interface A : B {}; interface B : C {}; interface C : A {};");
        let e = generate_stubs(&s, &["A", "B", "C"], "").unwrap_err();
        assert_eq!(
            e,
            GenError::InheritanceCycle {
                interface: "A".into(),
                chain: vec!["A".into(), "B".into(), "C".into(), "A".into()],
            }
        );
        assert_eq!(e.to_string(), "`A` has an inheritance cycle: A : B : C : A");
        // A cycle above the interface, not through it.
        let s = set("interface A : B {}; interface B : C {}; interface C : B {};");
        assert!(matches!(
            generate_stubs(&s, &["C", "B", "A"], "").unwrap_err(),
            GenError::InheritanceCycle { .. }
        ));
        // Long acyclic chains are fine.
        let src: String = (0..100)
            .map(|i| format!("interface I{} : I{} {{}};", i + 1, i))
            .chain(["interface I0 {};".to_owned()])
            .collect();
        let s = set(&src);
        let names: Vec<String> = (0..=100).map(|i| format!("I{i}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        assert!(generate_stubs(&s, &names, "").is_ok());
        let s = set("interface A { const octet X = 256; };");
        assert!(matches!(
            generate_stubs(&s, &["A"], "").unwrap_err(),
            GenError::BadConst { .. }
        ));
    }

    #[test]
    fn const_values() {
        let s = set(
            "interface A { const unsigned short H = 0x1F; const long O = 017; const long N = -3; const double I = -Infinity; const float F = .5; const boolean B = true; };",
        );
        let out = generate_stubs(&s, &["A"], "").unwrap();
        for line in [
            "pub const H: u16 = 31;",
            "pub const O: i32 = 15;",
            "pub const N: i32 = -3;",
            "pub const I: f64 = f64::NEG_INFINITY;",
            "pub const F: f32 = 0.5;",
            "pub const B: bool = true;",
        ] {
            assert!(out.contains(line), "{line}\n{out}");
        }
    }
}
