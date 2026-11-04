//! Binding glue from merged IDL interfaces, for `surimono-dom`.
//!
//! Stubs ([`crate::generate_stubs`]) give the host a trait per interface.
//! This module writes the two halves that connect those traits to
//! JavaScript, for the interfaces and members listed in a [`BindInterface`]
//! table:
//!
//! - **JavaScript** (`bindings.js`): interface objects and prototypes, one
//!   accessor or function per member, each with the brand check, the WebIDL
//!   argument conversions and the wrapping of the result. It runs inside the
//!   host's glue (`crates/dom/src/glue_head.js`), whose helpers it calls.
//! - **Rust** (`dispatch.rs`): one `match` from operation number to trait
//!   method. JavaScript crosses the boundary through a single native
//!   function, `call(op, generation, handle, ...args)`, with primitive
//!   arguments only.
//!
//! Only listed members exist in JavaScript. A member a page feature-detects
//! is then really absent, rather than present and throwing.
//!
//! As with stubs, the output depends only on its inputs: no maps with random
//! iteration order, no timestamps, no paths.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::IdlSet;
use crate::ast::*;
use crate::codegen::{Gen, GenError, rust_name};
use crate::model::{MergedInterface, MergedMember, NameKind};

/// An interface to bind and the members to expose. Constants are always
/// exposed; `"constructor"` names the constructor, and indexed and named
/// property getters, setters and deleters come with the interface when
/// their operations are listed (anonymous ones by `"named_getter"`,
/// `"named_setter"` and `"named_deleter"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindInterface {
    /// The interface's name; its parent must come earlier in the table.
    pub name: &'static str,
    /// The members to expose.
    pub members: &'static [&'static str],
}

/// Interfaces with no state in Rust. An event exists to call script
/// listeners, and those are JavaScript values: its fields and the dispatch
/// algorithm live in `crates/dom/src/glue_events.js`, under the memory limit,
/// in the wrapper's private record. The generator still writes the interface
/// from the IDL (names, constants, brand checks, conversions), and each member
/// calls `H.<Interface>_<member>(record, ...)`, with `H.set_...` for setters
/// and `H.new_<Interface>(new.target, ...)` for constructors.
const JS_INTERFACES: &[&str] = &[
    "EventTarget",
    "Event",
    "CustomEvent",
    "ErrorEvent",
    "Window",
];

/// A member name that isn't in the IDL. CSSOM defines
/// `CSSStyleProperties`' per-property attributes (`style.fontSize`) in prose,
/// one for each CSS property, so they are generated as named properties
/// answered by the host: `style_value` and `set_style_value`.
const STYLE_PROPERTIES: &str = "style_properties";

/// The generated glue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bindings {
    /// `crates/dom/src/generated/bindings.js`
    pub js: String,
    /// `crates/dom/src/generated/dispatch.rs`
    pub rust: String,
}

/// Operation 0: the handle's concrete interface number.
const OP_INTERFACE: u32 = 0;
/// Operation 1: an exotic object's supported property names.
const OP_NAMES: u32 = 1;
/// Operation 2: the page lifecycle moves `document.readyState`.
const OP_SET_READY_STATE: u32 = 2;
/// The first operation number for generated members.
const OP_FIRST: u32 = 3;

/// Members the glue calls itself, for event paths and handler attributes:
/// (interface, member). Each gets a `const OP_<member>` in `bindings.js`.
const GLUE_OPS: &[(&str, &str)] = &[
    ("Node", "parentNode"),
    ("Element", "localName"),
    ("Element", "getAttribute"),
    ("Element", "getAttributeNames"),
    ("Document", "body"),
];

/// IDL types of event handler IDL attributes (HTML §8.1.7.1).
const EVENT_HANDLER_TYPES: &[&str] = &[
    "EventHandler",
    "OnErrorEventHandler",
    "OnBeforeUnloadEventHandler",
];

/// Everything for one member's two halves.
struct Out {
    js: String,
    arms: String,
    next_op: u32,
    /// The operations of [`GLUE_OPS`] members bound so far.
    glue_ops: Vec<(&'static str, u32)>,
}

impl Out {
    fn op(&mut self, rust_call: &str) -> u32 {
        let op = self.next_op;
        self.next_op += 1;
        let _ = writeln!(self.arms, "        {op} => {rust_call}.map(Ret::from),");
        op
    }
}

/// `set`'s interfaces by name, to tell which types are bound.
fn bound_id(table: &[BindInterface], name: &str) -> Option<usize> {
    table.iter().position(|b| b.name == name)
}

/// The bound interface a value of IDL interface `name` is seen as: `name`
/// itself, or its nearest bound ancestor (`HTMLHeadElement` to
/// `HTMLElement`, while elements have no interfaces of their own).
fn bound_ancestor(set: &IdlSet, table: &[BindInterface], name: &str) -> Option<usize> {
    let mut at = name.to_owned();
    // Parent chains are short; the bound stops a cycle.
    for _ in 0..16 {
        if let Some(id) = bound_id(table, &at) {
            return Some(id);
        }
        at = set.interface(&at)?.parent?;
    }
    None
}

fn has_ext(attrs: &[ExtAttr], name: &str) -> bool {
    attrs.iter().any(|a| a.name == name)
}

fn is_string(n: &str) -> bool {
    matches!(n, "DOMString" | "USVString" | "ByteString" | "CSSOMString")
}

/// What a type is at the boundary.
enum Conv {
    /// `DOMString`-like; the flag is `[LegacyNullToEmptyString]`.
    Str(bool),
    Bool,
    /// A JS expression turning `x` into the integer type.
    Int(&'static str),
    /// A bound interface, by number.
    Obj(usize),
    /// `(Node or DOMString)`, for the variadic node methods.
    NodeOrStr,
    /// Not converted: passed as `undefined` (dictionaries, callbacks).
    Skip,
}

fn conv(gen_: &Gen<'_>, table: &[BindInterface], t: &Type, depth: u32) -> Option<Conv> {
    if depth > 8 {
        return None;
    }
    match &t.kind {
        TypeKind::Named(n) => {
            if is_string(n) {
                return Some(Conv::Str(has_ext(&t.ext_attrs, "LegacyNullToEmptyString")));
            }
            match n.as_str() {
                "boolean" => Some(Conv::Bool),
                "unsigned long" => Some(Conv::Int("x >>> 0")),
                "long" => Some(Conv::Int("x | 0")),
                "unsigned short" => Some(Conv::Int("(x >>> 0) & 65535")),
                "short" => Some(Conv::Int("(x << 16) >> 16")),
                other => {
                    if let Some(id) = bound_ancestor(gen_.set, table, other) {
                        return Some(Conv::Obj(id));
                    }
                    if let Some(target) = gen_.set.typedef(other) {
                        return conv(gen_, table, target, depth + 1);
                    }
                    Some(Conv::Skip)
                }
            }
        }
        TypeKind::Union(ts) => {
            // `(DOMString or ElementCreationOptions)`: options aren't read.
            let dictionary = ts.iter().any(|b| {
                matches!(&b.kind, TypeKind::Named(n)
                    if gen_.names.get(n.as_str()) == Some(&NameKind::Dictionary))
            });
            if dictionary {
                return Some(Conv::Skip);
            }
            // `(TrustedHTML or DOMString)` and the like: Trusted Types aren't
            // in scope, so the string branch is the type.
            let str_branch = ts
                .iter()
                .find(|b| matches!(&b.kind, TypeKind::Named(n) if is_string(n)));
            let node_branch = ts
                .iter()
                .any(|b| matches!(&b.kind, TypeKind::Named(n) if n == "Node"));
            match (node_branch, str_branch) {
                (true, Some(_)) => Some(Conv::NodeOrStr),
                (false, Some(b)) => conv(gen_, table, b, depth + 1),
                _ => None,
            }
        }
        _ => None,
    }
}

fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The JS expression converting `x` (an expression) to what `call` takes.
fn js_arg(c: &Conv, t: &Type, x: &str, g: &str) -> String {
    let body = match c {
        Conv::Str(_) => format!("str({x})"),
        Conv::Bool => format!("!!{x}"),
        Conv::Int(e) => format!("({})", e.replace('x', x)),
        Conv::Obj(id) => format!("obj({x}, {id}, {g})"),
        Conv::NodeOrStr => format!("nodeOrStr({x}, {g})"),
        Conv::Skip => return "undefined".to_owned(),
    };
    if let Conv::Str(true) = c {
        return format!("({x} === null ? '' : {body})");
    }
    if t.nullable {
        let none = if matches!(c, Conv::Obj(_)) {
            "0"
        } else {
            "null"
        };
        return format!("({x} === null || {x} === undefined ? {none} : {body})");
    }
    body
}

/// The JS default for an omitted optional argument.
fn js_default(d: &Option<DefaultValue>) -> String {
    match d {
        Some(DefaultValue::String(s)) => js_string(s),
        Some(DefaultValue::Const(c)) => c.to_string(),
        Some(DefaultValue::Null) => "null".to_owned(),
        _ => "undefined".to_owned(),
    }
}

/// `Args` accessor for a Rust parameter type from the stubs.
fn rust_arg(ty: &str, i: usize) -> Option<String> {
    let f = match ty {
        "String" => "string",
        "Option<String>" => "opt_string",
        "bool" => "boolean",
        "Option<bool>" => "opt_boolean",
        "u32" => "uint",
        "i32" => "int",
        "u16" => "ushort",
        "i16" => "short",
        "ObjectId" => "id",
        "Option<ObjectId>" => "opt_id",
        "Value" => "value",
        "Vec<String>" => "strings",
        "Vec<Value>" => "values",
        _ => return None,
    };
    Some(format!("a.{f}({i})?"))
}

/// The code for an argument list.
struct ArgCode {
    /// The JS function's declared parameters (the required arguments).
    params: Vec<String>,
    /// A JS expression per non-variadic argument.
    js: Vec<String>,
    /// A Rust expression per argument.
    rust: Vec<String>,
    /// The conversion of a variadic last argument.
    variadic: Option<String>,
}

struct Bound<'a, 'g> {
    gen_: &'a mut Gen<'g>,
    table: &'a [BindInterface],
    iface: &'a MergedInterface,
    id: usize,
    out: &'a mut Out,
    /// The members are hand-written in the glue (see [`JS_INTERFACES`]).
    js_impl: bool,
}

/// Operations sharing `name` in `m`, for the overload check.
fn overloads<'m>(m: &'m MergedInterface, name: &str) -> Vec<&'m MergedMember> {
    m.members
        .iter()
        .filter(|mm| {
            matches!(&mm.member.kind, MemberKind::Operation { name: Some(n), .. } if n == name)
        })
        .collect()
}

impl Bound<'_, '_> {
    /// Keeps `op` for the glue if it is one of [`GLUE_OPS`].
    fn note_glue_op(&mut self, member: &str, op: u32) {
        let found = GLUE_OPS
            .iter()
            .find(|(i, m)| *i == self.iface.name && *m == member);
        if let Some((_, m)) = found {
            self.out.glue_ops.push((m, op));
        }
    }

    /// The interface's event handler IDL attributes, including its mixins'.
    /// The glue defines them all: they only need a name.
    fn event_handlers(&mut self) {
        let names: Vec<String> = self
            .iface
            .members
            .iter()
            .filter_map(|m| match &m.member.kind {
                MemberKind::Attribute { name, ty, .. }
                    if matches!(&ty.kind, TypeKind::Named(n)
                        if EVENT_HANDLER_TYPES.contains(&n.as_str())) =>
                {
                    Some(js_string(name))
                }
                _ => None,
            })
            .collect();
        if !names.is_empty() {
            let _ = writeln!(
                self.out.js,
                "handlers({}, [{}]);",
                self.id,
                names.join(", ")
            );
        }
    }

    fn unsupported(&self, member: &str, ty: &Type) -> GenError {
        GenError::UnsupportedType {
            interface: self.iface.name.clone(),
            member: member.to_owned(),
            ty: ty.to_string(),
        }
    }

    fn trait_call(&self, method: &str, args: &[String], has_this: bool) -> String {
        let mut all = vec!["h".to_owned()];
        if has_this {
            all.push("this".to_owned());
        }
        all.extend(args.iter().cloned());
        format!("idl::{}::{method}({})", self.iface.name, all.join(", "))
    }

    /// JS that turns the result expression `r` into what scripts see.
    fn js_return(&self, member: &str, t: &Type, r: &str, g: &str) -> Result<String, GenError> {
        match &t.kind {
            TypeKind::Named(n)
                if is_string(n)
                    || matches!(
                        n.as_str(),
                        "undefined"
                            | "boolean"
                            | "unsigned long"
                            | "long"
                            | "unsigned short"
                            | "short"
                    ) =>
            {
                Ok(r.to_owned())
            }
            TypeKind::Union(ts)
                if ts
                    .iter()
                    .any(|b| matches!(&b.kind, TypeKind::Named(n) if is_string(n))) =>
            {
                Ok(r.to_owned())
            }
            TypeKind::Named(n) if self.gen_.names.get(n.as_str()) == Some(&NameKind::Enum) => {
                Ok(r.to_owned())
            }
            TypeKind::Sequence(e) if matches!(&e.kind, TypeKind::Named(n) if is_string(n)) => {
                Ok(format!("nameArray({r})"))
            }
            _ => match conv(self.gen_, self.table, t, 0) {
                Some(Conv::Obj(_)) => Ok(format!("wrap({g}, {r})")),
                Some(Conv::Str(_) | Conv::Bool | Conv::Int(_)) => Ok(r.to_owned()),
                _ => Err(self.unsupported(member, t)),
            },
        }
    }

    fn attribute(
        &mut self,
        name: &str,
        ty: &Type,
        readonly: bool,
        ext_attrs: &[ExtAttr],
    ) -> Result<(), GenError> {
        let id = self.id;
        if self.js_impl {
            return self.js_attribute(name, ty, readonly);
        }
        let rust = rust_name(name);
        let getter = self.trait_call(&rust, &[], true);
        let op = self.out.op(&getter);
        self.note_glue_op(name, op);
        let ret = self.js_return(name, ty, "r", "s.g")?;
        let _ = write!(
            self.out.js,
            "attr({id}, {}, function () {{ const s = self(this, {id}); const r = call({op}, s.g, s.h); return {ret}; }}",
            js_string(name)
        );
        let forwards = ext_attrs.iter().find(|a| a.name == "PutForwards");
        if !readonly {
            let c = conv(self.gen_, self.table, ty, 0).ok_or_else(|| self.unsupported(name, ty))?;
            let rt = self.gen_.rust_type(ty, 0)?;
            let arg = rust_arg(&rt, 0).ok_or_else(|| self.unsupported(name, ty))?;
            let setter = format!("set_{}", rust.trim_end_matches('_'));
            let setter = self.trait_call(&setter, &[arg], true);
            let op = self.out.op(&setter);
            let x = js_arg(&c, ty, "v", "s.g");
            let _ = write!(
                self.out.js,
                ", function (v) {{ const s = self(this, {id}); call({op}, s.g, s.h, {x}); }}"
            );
        } else if let Some(fwd) = forwards {
            let ExtAttrValue::Ident(target) = &fwd.value else {
                return Err(self.unsupported(name, ty));
            };
            let _ = write!(
                self.out.js,
                ", function (v) {{ self(this, {id}); this.{name}.{target} = v; }}"
            );
        }
        self.out.js.push_str(");\n");
        Ok(())
    }

    fn arguments(&mut self, member: &str, args: &[Argument], g: &str) -> Result<ArgCode, GenError> {
        let params = self.gen_.params(args)?;
        let mut js_params = Vec::new();
        let mut js_args = Vec::new();
        let mut rust_args = Vec::new();
        let mut variadic = None;
        for (i, (a, (_, rt))) in args.iter().zip(&params).enumerate() {
            let mut c = conv(self.gen_, self.table, &a.ty, 0)
                .ok_or_else(|| self.unsupported(member, &a.ty))?;
            // On a required argument the parser hangs it on the argument, on
            // an optional one on the type.
            if let Conv::Str(null_to_empty) = &mut c {
                *null_to_empty |= has_ext(&a.ext_attrs, "LegacyNullToEmptyString");
            }
            rust_args.push(if matches!(c, Conv::Skip) {
                "Value::Undefined".to_owned()
            } else {
                rust_arg(rt, i).ok_or_else(|| self.unsupported(member, &a.ty))?
            });
            if a.variadic {
                if i != 0 {
                    return Err(self.unsupported(member, &a.ty));
                }
                variadic = Some(match c {
                    Conv::Str(_) => "str".to_owned(),
                    Conv::NodeOrStr => "nodeOrStr".to_owned(),
                    _ => return Err(self.unsupported(member, &a.ty)),
                });
                continue;
            }
            if a.optional {
                let x = format!("(argc > {i} ? arguments[{i}] : undefined)");
                let v = format!("o{i}");
                let conv_expr = js_arg(&c, &a.ty, &v, g);
                let default = js_default(&a.default);
                js_args.push(format!(
                    "((({v}) => {v} === undefined ? {default} : {conv_expr})({x}))"
                ));
            } else {
                let p = format!("a{i}");
                js_args.push(js_arg(&c, &a.ty, &p, g));
                js_params.push(p);
            }
        }
        Ok(ArgCode {
            params: js_params,
            js: js_args,
            rust: rust_args,
            variadic,
        })
    }

    /// A member of a [`JS_INTERFACES`] interface: the IDL's name and
    /// conversion, the glue's body.
    fn js_attribute(&mut self, name: &str, ty: &Type, readonly: bool) -> Result<(), GenError> {
        let (id, iface) = (self.id, self.iface.name.clone());
        let _ = write!(
            self.out.js,
            "attr({id}, {}, function () {{ return H.{iface}_{name}(self(this, {id})); }}",
            js_string(name)
        );
        if !readonly {
            let x = self.js_impl_arg(ty, "v");
            let _ = write!(
                self.out.js,
                ", function (v) {{ H.set_{iface}_{name}(self(this, {id}), {x}); }}"
            );
        }
        self.out.js.push_str(");\n");
        Ok(())
    }

    /// `x` converted for a hand-written member: as for a native one, except
    /// that bound objects stay records and what `conv` leaves alone
    /// (dictionaries, callbacks, `any`) arrives as it came.
    fn js_impl_arg(&self, ty: &Type, x: &str) -> String {
        match conv(self.gen_, self.table, ty, 0) {
            Some(Conv::Obj(id)) if ty.nullable => {
                format!("({x} === null || {x} === undefined ? null : self({x}, {id}))")
            }
            Some(Conv::Obj(id)) => format!("self({x}, {id})"),
            Some(c @ (Conv::Str(_) | Conv::Bool | Conv::Int(_))) => js_arg(&c, ty, x, ""),
            _ => x.to_owned(),
        }
    }

    /// Declared parameters and argument expressions of a hand-written
    /// member. An omitted optional argument takes its default, undefined
    /// for a dictionary; the glue reads dictionaries itself.
    fn js_impl_arguments(&self, args: &[Argument]) -> (Vec<String>, Vec<String>) {
        let mut params = Vec::new();
        let mut exprs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            if a.optional {
                let v = format!("o{i}");
                let x = format!("(argc > {i} ? arguments[{i}] : undefined)");
                let converted = self.js_impl_arg(&a.ty, &v);
                let default = js_default(&a.default);
                exprs.push(format!(
                    "((({v}) => {v} === undefined ? {default} : {converted})({x}))"
                ));
            } else {
                let p = format!("a{i}");
                exprs.push(self.js_impl_arg(&a.ty, &p));
                params.push(p);
            }
        }
        (params, exprs)
    }

    fn js_operation(&mut self, name: &str, args: &[Argument]) {
        let (id, iface) = (self.id, self.iface.name.clone());
        let (params, exprs) = self.js_impl_arguments(args);
        let required = params.len();
        let mut body = format!("const s = self(this, {id}); const argc = arguments.length; ");
        if required > 0 {
            let _ = write!(body, "need(argc, {required}, {}); ", js_string(name));
        }
        let mut all = vec!["s".to_owned()];
        all.extend(exprs);
        let _ = writeln!(
            self.out.js,
            "op({id}, {}, function {name}({}) {{ {body}return H.{iface}_{name}({}); }});",
            js_string(name),
            params.join(", "),
            all.join(", ")
        );
    }

    fn operation(&mut self, name: &str) -> Result<(), GenError> {
        let found = overloads(self.iface, name);
        let [mm] = found.as_slice() else {
            if found.is_empty() {
                return Err(self.unknown(name));
            }
            return Err(GenError::Overloaded {
                interface: self.iface.name.clone(),
                member: name.to_owned(),
            });
        };
        let MemberKind::Operation {
            return_type, args, ..
        } = &mm.member.kind
        else {
            return Err(self.unknown(name));
        };
        let id = self.id;
        if self.js_impl {
            self.js_operation(name, args);
            return Ok(());
        }
        let ArgCode {
            params,
            js: js_args,
            rust: rust_args,
            variadic,
        } = self.arguments(name, args, "s.g")?;
        let call = self.trait_call(&rust_name(name), &rust_args, true);
        let op = self.out.op(&call);
        self.note_glue_op(name, op);
        let ret = self.js_return(name, return_type, "r", "s.g")?;
        let required = params.len();
        let mut body = format!("const s = self(this, {id}); const argc = arguments.length; ");
        if required > 0 {
            let _ = write!(body, "need(argc, {required}, {}); ", js_string(name));
        }
        let call_expr = match variadic {
            Some(c) => format!("vcall({op}, s, arguments, {c})"),
            None => {
                let mut all = vec![op.to_string(), "s.g".to_owned(), "s.h".to_owned()];
                all.extend(js_args);
                format!("call({})", all.join(", "))
            }
        };
        let _ = writeln!(
            self.out.js,
            "op({id}, {}, function {name}({}) {{ {body}const r = {call_expr}; return {ret}; }});",
            js_string(name),
            params.join(", ")
        );
        Ok(())
    }

    fn unknown(&self, member: &str) -> GenError {
        GenError::UnknownMember {
            interface: self.iface.name.clone(),
            member: member.to_owned(),
        }
    }

    /// The constructor function's source, or `null`.
    fn constructor(&mut self, listed: &[&str]) -> Result<String, GenError> {
        if !listed.contains(&"constructor") {
            return Ok("null".to_owned());
        }
        let ctors: Vec<_> = self
            .iface
            .members
            .iter()
            .filter_map(|m| match &m.member.kind {
                MemberKind::Constructor { args } => Some(args.clone()),
                _ => None,
            })
            .collect();
        let [args] = ctors.as_slice() else {
            return Err(self.unknown("constructor"));
        };
        if self.js_impl {
            let (params, exprs) = self.js_impl_arguments(args);
            let required = params.len();
            let name = &self.iface.name;
            let check = if required > 0 {
                format!("need(argc, {required}, {}); ", js_string(name))
            } else {
                String::new()
            };
            let mut all = vec!["new.target".to_owned()];
            all.extend(exprs);
            return Ok(format!(
                "function {name}({}) {{ if (new.target === undefined) throw new TE('Constructor requires new'); const argc = arguments.length; {check}return H.new_{name}({}); }}",
                params.join(", "),
                all.join(", ")
            ));
        }
        let ArgCode {
            params,
            js: js_args,
            rust: rust_args,
            ..
        } = self.arguments("constructor", args, "g")?;
        let call = self.trait_call("constructor", &rust_args, false);
        let op = self.out.op(&call);
        let mut all = vec![op.to_string(), "g".to_owned(), "root()".to_owned()];
        all.extend(js_args);
        Ok(format!(
            "function {}({}) {{ if (new.target === undefined) throw new TE('Constructor requires new'); const argc = arguments.length; const g = current(); return adopt(wrap(g, call({})), new.target, {}); }}",
            self.iface.name,
            params.join(", "),
            all.join(", "),
            self.id
        ))
    }

    /// The object literal for `exotic()`, if the interface has property
    /// getters: the operations behind `list[0]` and `map.name`.
    fn exotic(&mut self, listed: &[&str]) -> Result<Option<String>, GenError> {
        let mut index = None;
        let mut named = None;
        let mut setter = None;
        let mut deleter = None;
        for mm in &self.iface.members {
            let MemberKind::Operation {
                name,
                special: Some(sp),
                args,
                return_type,
                ..
            } = &mm.member.kind
            else {
                continue;
            };
            let key = match (name, sp) {
                (Some(n), _) => n.clone(),
                (None, Special::Getter) => "named_getter".to_owned(),
                (None, Special::Setter) => "named_setter".to_owned(),
                (None, Special::Deleter) => "named_deleter".to_owned(),
            };
            if !listed.contains(&key.as_str()) {
                continue;
            }
            let by_index = args
                .first()
                .is_some_and(|a| a.ty.kind == TypeKind::Named("unsigned long".into()));
            let rust = rust_name(&key);
            let anonymous_getter = name.is_none() && matches!(sp, Special::Getter);
            let (rust_args, entry): (Vec<String>, _) = match sp {
                Special::Getter if by_index => (vec!["a.uint(0)?".into()], &mut index),
                Special::Getter => (vec!["a.string(0)?".into()], &mut named),
                Special::Setter => (
                    vec!["a.string(0)?".into(), "a.string(1)?".into()],
                    &mut setter,
                ),
                Special::Deleter => (vec!["a.string(0)?".into()], &mut deleter),
            };
            let wraps = matches!(&return_type.kind, TypeKind::Named(n) if bound_id(self.table, n).is_some());
            *entry = Some((rust, rust_args, wraps, anonymous_getter));
        }
        let style = listed.contains(&STYLE_PROPERTIES);
        if index.is_none() && named.is_none() && !style {
            return Ok(None);
        }
        let length = self.iface.members.iter().any(
            |m| matches!(&m.member.kind, MemberKind::Attribute { name, .. } if name == "length"),
        );
        let mut fields = Vec::new();
        if length {
            let call = self.trait_call("length", &[], true);
            fields.push(format!("length: {}", self.out.op(&call)));
        }
        for (key, slot) in [
            ("item", &index),
            ("named", &named),
            ("set", &setter),
            ("del", &deleter),
        ] {
            if let Some((rust, args, wraps, anonymous_getter)) = slot {
                // The stub returns a plain string, with no way to say
                // "no such property"; this host method can.
                let call = if *anonymous_getter {
                    format!("h.named_value(this, {})", args.join(", "))
                } else {
                    self.trait_call(rust, args, true)
                };
                fields.push(format!("{key}: {}", self.out.op(&call)));
                if key != "set" && key != "del" {
                    fields.push(format!("{key}Wraps: {wraps}"));
                }
            }
        }
        if style {
            let get = self.out.op("h.style_value(this, a.string(0)?)");
            let set = self
                .out
                .op("h.set_style_value(this, a.string(0)?, a.string(1)?)");
            fields.push(format!(
                "named: {get}, namedWraps: false, set: {set}, knownOnly: true"
            ));
        }
        if has_ext(&self.iface.ext_attrs, "LegacyOverrideBuiltIns") {
            fields.push("overrides: true".to_owned());
        }
        if has_ext(&self.iface.ext_attrs, "LegacyUnenumerableNamedProperties") {
            fields.push("unenumerable: true".to_owned());
        }
        Ok(Some(format!("{{ {} }}", fields.join(", "))))
    }

    fn interface(&mut self, spec: &BindInterface) -> Result<(), GenError> {
        let id = self.id;
        let parent = match &self.iface.parent {
            Some(p) => bound_id(self.table, p)
                .filter(|&i| i < id)
                .ok_or_else(|| GenError::ParentNotGenerated {
                    interface: spec.name.to_owned(),
                    parent: p.clone(),
                })?
                .to_string(),
            None => "-1".to_owned(),
        };
        let _ = writeln!(self.out.js, "\n// {}", spec.name);
        let ctor = self.constructor(spec.members)?;
        let _ = writeln!(
            self.out.js,
            "iface({id}, {}, {parent}, {ctor});",
            js_string(spec.name)
        );
        for mm in &self.iface.members.clone() {
            if let MemberKind::Const { name, value, .. } = &mm.member.kind {
                let _ = writeln!(self.out.js, "konst({id}, {}, {value});", js_string(name));
            }
        }
        let mut seen = BTreeSet::new();
        for &name in spec.members {
            if !seen.insert(name) {
                return Err(GenError::Duplicate(format!("{}.{name}", spec.name)));
            }
            let (name, getter_only) = match name.strip_suffix(GETTER_ONLY) {
                Some(n) => (n, true),
                None => (name, false),
            };
            if matches!(
                name,
                "constructor"
                    | "named_getter"
                    | "named_setter"
                    | "named_deleter"
                    | "iterable"
                    | STYLE_PROPERTIES
            ) {
                continue;
            }
            let attr = self
                .iface
                .members
                .iter()
                .find_map(|m| match &m.member.kind {
                    MemberKind::Attribute {
                        name: n,
                        ty,
                        readonly,
                        ..
                    } if n == name => Some((ty.clone(), *readonly, m.member.ext_attrs.clone())),
                    _ => None,
                });
            match attr {
                Some((ty, readonly, ext)) => {
                    self.attribute(name, &ty, readonly || getter_only, &ext)?
                }
                None => self.operation(name)?,
            }
        }
        if spec.members.contains(&"iterable") {
            if !self
                .iface
                .members
                .iter()
                .any(|m| matches!(m.member.kind, MemberKind::Iterable { .. }))
            {
                return Err(self.unknown("iterable"));
            }
            let _ = writeln!(self.out.js, "iterable({id});");
        }
        let stringifies = self
            .iface
            .members
            .iter()
            .find_map(|m| match &m.member.kind {
                MemberKind::Attribute {
                    name,
                    stringifier: true,
                    ..
                } => Some(name.clone()),
                _ => None,
            });
        if let Some(s) = stringifies.filter(|s| spec.members.contains(&s.as_str())) {
            let _ = writeln!(self.out.js, "stringifier({id}, {});", js_string(&s));
        }
        if let Some(x) = self.exotic(spec.members)? {
            let _ = writeln!(self.out.js, "exotic({id}, {x});");
        }
        self.event_handlers();
        Ok(())
    }
}

/// Generates the glue for `table`, whose interfaces come parents first.
/// `header` becomes a comment at the top of each file.
pub fn generate_bindings(
    set: &IdlSet,
    table: &[BindInterface],
    header: &str,
) -> Result<Bindings, GenError> {
    if table.len() > 31 {
        return Err(GenError::TooManyInterfaces);
    }
    let mut gen_ = Gen {
        set,
        names: set.names(),
        used_value: false,
    };
    let mut out = Out {
        js: String::new(),
        arms: String::new(),
        next_op: OP_FIRST,
        glue_ops: Vec::new(),
    };
    for (id, spec) in table.iter().enumerate() {
        if bound_id(table, spec.name) != Some(id) {
            return Err(GenError::Duplicate(spec.name.to_owned()));
        }
        let iface = set
            .interface(spec.name)
            .ok_or_else(|| GenError::UnknownInterface(spec.name.to_owned()))?;
        Bound {
            gen_: &mut gen_,
            table,
            iface: &iface,
            id,
            out: &mut out,
            js_impl: JS_INTERFACES.contains(&spec.name),
        }
        .interface(spec)?;
    }

    let note = |prefix: &str| {
        let mut s = String::new();
        for line in header.lines() {
            let _ = writeln!(s, "{prefix} {line}");
        }
        let _ = writeln!(
            s,
            "{prefix} @generated by surimono-webidl. Do not edit by hand; regenerate with\n{prefix} `cargo run -p surimono-webidl --example regen`."
        );
        s
    };

    let mut js = note("//");
    if let Some(id) = bound_id(table, "Node") {
        let _ = writeln!(js, "const nodeBit = 1 << {id};");
    }
    for name in JS_INTERFACES.iter().chain(&["HTMLElement", "Document"]) {
        if let Some(id) = bound_id(table, name) {
            let _ = writeln!(js, "const I_{name} = {id};");
        }
    }
    js.push_str(&out.js);

    let _ = writeln!(js, "const OP_setReadyState = {OP_SET_READY_STATE};");
    for (member, op) in &out.glue_ops {
        let _ = writeln!(js, "const OP_{member} = {op};");
    }
    let mut rust = note("//");
    rust.push_str("\nuse surimono_webidl::runtime::{BindingError, BindingResult, ObjectId};\n");
    if out.arms.contains("Value::Undefined") {
        rust.push_str("use surimono_webidl::runtime::Value;\n");
    }
    rust.push_str("use surimono_webidl::stubs as idl;\n\n");
    rust.push_str("use crate::Document;\nuse crate::bindings::{Args, Ret};\n\n");
    rust.push_str("/// The number of each bound interface, as `wrap` receives it.\n");
    rust.push_str("#[allow(dead_code)]\npub(crate) mod interface {\n");
    for (id, spec) in table.iter().enumerate() {
        let c = rust_name(spec.name).to_ascii_uppercase();
        let _ = writeln!(rust, "    pub(crate) const {c}: u32 = {id};");
    }
    rust.push_str("}\n\n");
    rust.push_str("/// Runs operation `op` on `this`, with `a` from JavaScript.\n");
    rust.push_str("pub(crate) fn dispatch(\n    h: &mut Document,\n    op: u32,\n    this: ObjectId,\n    a: &Args<'_>,\n) -> BindingResult<Ret> {\n    match op {\n");
    let _ = writeln!(
        rust,
        "        {OP_INTERFACE} => h.interface_of(this).map(Ret::from),"
    );
    let _ = writeln!(
        rust,
        "        {OP_NAMES} => h.supported_names(this).map(Ret::from),"
    );
    let _ = writeln!(
        rust,
        "        {OP_SET_READY_STATE} => h.set_ready_state(a.uint(0)?).map(Ret::from),"
    );
    rust.push_str(&out.arms);
    rust.push_str(
        "        _ => Err(BindingError::TypeError(\"unknown operation\".to_owned())),\n    }\n}\n",
    );
    Ok(Bindings { js, rust })
}

/// After a writable attribute's name in a table: expose the getter only, for
/// a member whose setter isn't implemented yet.
const GETTER_ONLY: &str = ":getter";

/// What `surimono-dom` exposes, parents first. Every name must be a member
/// of the interface in the vendored IDL (including mixins and partials).
pub const DOM_BINDINGS: &[BindInterface] = &[
    BindInterface {
        name: "EventTarget",
        members: &[
            "constructor",
            "addEventListener",
            "removeEventListener",
            "dispatchEvent",
        ],
    },
    BindInterface {
        name: "Event",
        members: &[
            "constructor",
            "type",
            "target",
            "srcElement",
            "currentTarget",
            "composedPath",
            "eventPhase",
            "stopPropagation",
            "cancelBubble",
            "stopImmediatePropagation",
            "bubbles",
            "cancelable",
            "returnValue",
            "preventDefault",
            "defaultPrevented",
            "composed",
            "isTrusted",
            "timeStamp",
            "initEvent",
        ],
    },
    BindInterface {
        name: "CustomEvent",
        members: &["constructor", "detail", "initCustomEvent"],
    },
    BindInterface {
        name: "ErrorEvent",
        members: &[
            "constructor",
            "message",
            "filename",
            "lineno",
            "colno",
            "error",
        ],
    },
    BindInterface {
        name: "Window",
        members: &["devicePixelRatio"],
    },
    BindInterface {
        name: "Node",
        members: &[
            "nodeType",
            "nodeName",
            "isConnected",
            "ownerDocument",
            "parentNode",
            "parentElement",
            "hasChildNodes",
            "firstChild",
            "lastChild",
            "previousSibling",
            "nextSibling",
            "nodeValue",
            "textContent",
            "getRootNode",
            "childNodes",
            "normalize",
            "cloneNode",
            "isEqualNode",
            "isSameNode",
            "compareDocumentPosition",
            "contains",
            "insertBefore",
            "appendChild",
            "replaceChild",
            "removeChild",
        ],
    },
    BindInterface {
        name: "Element",
        members: &[
            "namespaceURI",
            "prefix",
            "localName",
            "tagName",
            "id",
            "className",
            "classList",
            "attributes",
            "hasAttributes",
            "getAttributeNames",
            "getAttribute",
            "setAttribute",
            "removeAttribute",
            "toggleAttribute",
            "hasAttribute",
            "getAttributeNS",
            "setAttributeNS",
            "removeAttributeNS",
            "hasAttributeNS",
            "closest",
            "matches",
            "getElementsByTagName",
            "getElementsByClassName",
            "querySelector",
            "querySelectorAll",
            "innerHTML",
            "outerHTML",
            "insertAdjacentHTML",
            "insertAdjacentElement",
            "insertAdjacentText",
            "children",
            "firstElementChild",
            "lastElementChild",
            "childElementCount",
            "prepend",
            "append",
            "replaceChildren",
            "previousElementSibling",
            "nextElementSibling",
            "before",
            "after",
            "replaceWith",
            "remove",
        ],
    },
    BindInterface {
        name: "HTMLElement",
        members: &["title", "lang", "dataset", "style"],
    },
    BindInterface {
        name: "SVGElement",
        members: &["style"],
    },
    BindInterface {
        name: "Document",
        members: &[
            "documentElement",
            "head",
            "body:getter",
            "title",
            "readyState",
            "compatMode",
            "characterSet",
            "contentType",
            "getElementById",
            "getElementsByTagName",
            "getElementsByClassName",
            "querySelector",
            "querySelectorAll",
            "createElement",
            "createElementNS",
            "createDocumentFragment",
            "createTextNode",
            "createComment",
            "adoptNode",
            "children",
            "firstElementChild",
            "lastElementChild",
            "childElementCount",
            "prepend",
            "append",
            "replaceChildren",
        ],
    },
    BindInterface {
        name: "DocumentFragment",
        members: &[
            "constructor",
            "getElementById",
            "querySelector",
            "querySelectorAll",
            "children",
            "firstElementChild",
            "lastElementChild",
            "childElementCount",
            "prepend",
            "append",
            "replaceChildren",
        ],
    },
    BindInterface {
        name: "CharacterData",
        members: &[
            "data",
            "length",
            "substringData",
            "appendData",
            "insertData",
            "deleteData",
            "replaceData",
            "previousElementSibling",
            "nextElementSibling",
            "before",
            "after",
            "replaceWith",
            "remove",
        ],
    },
    BindInterface {
        name: "Text",
        members: &["constructor", "splitText", "wholeText"],
    },
    BindInterface {
        name: "Comment",
        members: &["constructor"],
    },
    BindInterface {
        name: "Attr",
        members: &[
            "namespaceURI",
            "prefix",
            "localName",
            "name",
            "value",
            "ownerElement",
            "specified",
        ],
    },
    BindInterface {
        name: "NodeList",
        members: &["item", "length", "iterable"],
    },
    BindInterface {
        name: "HTMLCollection",
        members: &["length", "item", "namedItem"],
    },
    BindInterface {
        name: "NamedNodeMap",
        members: &[
            "length",
            "item",
            "getNamedItem",
            "getNamedItemNS",
            "removeNamedItem",
            "removeNamedItemNS",
        ],
    },
    BindInterface {
        name: "DOMTokenList",
        members: &[
            "length", "item", "contains", "add", "remove", "toggle", "replace", "supports",
            "value", "iterable",
        ],
    },
    BindInterface {
        name: "DOMStringMap",
        members: &["named_getter", "named_setter", "named_deleter"],
    },
    // Only for `parentRule`, always null on an inline style.
    BindInterface {
        name: "CSSRule",
        members: &[],
    },
    BindInterface {
        name: "CSSStyleDeclaration",
        members: &[
            "cssText",
            "parentRule",
            "length",
            "item",
            "getPropertyValue",
            "getPropertyPriority",
            "setProperty",
            "removeProperty",
            STYLE_PROPERTIES,
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn set(src: &str) -> IdlSet {
        IdlSet::parse_sources([("t.idl", src)]).unwrap()
    }

    fn bind(
        s: &IdlSet,
        name: &'static str,
        members: &'static [&'static str],
    ) -> Result<Bindings, GenError> {
        generate_bindings(s, &[BindInterface { name, members }], "h")
    }

    #[test]
    fn listed_members_only() {
        let s = set(
            "interface A { attribute long x; readonly attribute DOMString y; boolean z(DOMString p, optional boolean q = false); };",
        );
        let b = bind(&s, "A", &["y", "z"]).unwrap();
        assert!(b.js.contains(r#"attr(0, "y","#), "{}", b.js);
        assert!(b.js.contains(r#"op(0, "z", function z(a0) {"#), "{}", b.js);
        assert!(!b.js.contains('x') || !b.js.contains(r#""x""#));
        assert!(
            b.rust.contains("3 => idl::A::y(h, this).map(Ret::from),"),
            "{}",
            b.rust
        );
        assert!(
            b.rust
                .contains("4 => idl::A::z(h, this, a.string(0)?, a.boolean(1)?).map(Ret::from),"),
            "{}",
            b.rust
        );
    }

    #[test]
    fn errors_are_typed() {
        let s = set(
            "interface A { undefined f(); undefined f(long x); sequence<long> g(); }; interface B : A {};",
        );
        assert_eq!(
            bind(&s, "A", &["nope"]).unwrap_err(),
            GenError::UnknownMember {
                interface: "A".into(),
                member: "nope".into()
            }
        );
        assert_eq!(
            bind(&s, "A", &["f"]).unwrap_err(),
            GenError::Overloaded {
                interface: "A".into(),
                member: "f".into()
            }
        );
        assert!(matches!(
            bind(&s, "A", &["g"]).unwrap_err(),
            GenError::UnsupportedType { .. }
        ));
        assert!(matches!(
            bind(&s, "B", &[]).unwrap_err(),
            GenError::ParentNotGenerated { .. }
        ));
        assert!(matches!(
            bind(&s, "Z", &[]).unwrap_err(),
            GenError::UnknownInterface(_)
        ));
    }

    #[test]
    fn legacy_null_and_put_forwards() {
        let s = set(
            "interface L { attribute DOMString value; }; interface A { attribute [LegacyNullToEmptyString] DOMString d; [PutForwards=value] readonly attribute L l; };",
        );
        let b = generate_bindings(
            &s,
            &[
                BindInterface {
                    name: "L",
                    members: &["value"],
                },
                BindInterface {
                    name: "A",
                    members: &["d", "l"],
                },
            ],
            "h",
        )
        .unwrap();
        assert!(b.js.contains("(v === null ? '' : str(v))"), "{}", b.js);
        assert!(b.js.contains("this.l.value = v;"), "{}", b.js);
    }

    #[test]
    fn vendored_table_generates() {
        crate::generate_vendored_bindings().unwrap();
    }
}
