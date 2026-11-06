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
///
/// Every interface that inherits from `Event` is one of these too, without
/// being listed, and more of it is generated: the constructor, and each
/// attribute with a member of the same name in the init dictionary (DOM §2.5
/// "inner event creation steps"). Its dictionary becomes a `D_<name>` function
/// that reads and converts the members in WebIDL's order.
const JS_INTERFACES: &[&str] = &[
    "EventTarget",
    "Event",
    "Window",
    "MutationObserver",
    "DOMRectReadOnly",
    "DOMRect",
    "DOMRectList",
    "DOMParser",
    "XMLSerializer",
    "AbortController",
    "AbortSignal",
    "Performance",
    "Location",
    "History",
    "StaticRange",
    "URL",
    "URLSearchParams",
    "TextEncoder",
    "TextDecoder",
    "Navigator",
    "PluginArray",
    "MimeTypeArray",
    "CustomElementRegistry",
    "HTMLTableElement",
    "HTMLTableSectionElement",
    "HTMLTableRowElement",
    "HTMLTableCellElement",
    "HTMLSelectElement",
    "HTMLOptionElement",
    "HTMLOptionsCollection",
    "NodeIterator",
    "TreeWalker",
    "CSSRule",
    "CSSGroupingRule",
    "CSSStyleRule",
    "StyleSheet",
    "CSSStyleSheet",
    "StyleSheetList",
    "CSSRuleList",
];

/// Single members of interfaces otherwise in Rust that live in the glue:
/// `H.<Interface>_<member>`, as for [`JS_INTERFACES`].
const JS_MEMBERS: &[(&str, &str)] = &[
    ("Document", "createEvent"),
    ("Document", "rootElement"),
    ("HTMLElement", "click"),
    ("HTMLInputElement", "form"),
    ("HTMLInputElement", "indeterminate"),
    ("HTMLButtonElement", "form"),
    ("HTMLButtonElement", "type"),
    ("HTMLTextAreaElement", "form"),
    ("HTMLFieldSetElement", "form"),
    ("HTMLOutputElement", "form"),
    ("HTMLLabelElement", "form"),
    ("HTMLLabelElement", "control"),
    ("HTMLFormElement", "submit"),
    ("HTMLFormElement", "reset"),
    ("HTMLFormElement", "requestSubmit"),
    ("HTMLFormElement", "action"),
    ("HTMLFormElement", "method"),
    ("HTMLFormElement", "enctype"),
    ("HTMLFormElement", "encoding"),
    ("HTMLFormElement", "elements"),
    ("HTMLFormElement", "length"),
    ("HTMLTextAreaElement", "value"),
    ("HTMLTextAreaElement", "defaultValue"),
    ("Document", "cookie"),
    ("Document", "styleSheets"),
    ("HTMLStyleElement", "sheet"),
    ("HTMLLinkElement", "sheet"),
    ("Document", "createNodeIterator"),
    ("Document", "createTreeWalker"),
    ("Document", "activeElement"),
    ("Document", "hasFocus"),
    ("HTMLElement", "focus"),
    ("HTMLElement", "blur"),
    ("HTMLElement", "tabIndex"),
    ("SVGElement", "focus"),
    ("SVGElement", "blur"),
    ("SVGElement", "tabIndex"),
    ("HTMLAnchorElement", "origin"),
    ("HTMLAnchorElement", "protocol"),
    ("HTMLAnchorElement", "username"),
    ("HTMLAnchorElement", "password"),
    ("HTMLAnchorElement", "host"),
    ("HTMLAnchorElement", "hostname"),
    ("HTMLAnchorElement", "port"),
    ("HTMLAnchorElement", "pathname"),
    ("HTMLAnchorElement", "search"),
    ("HTMLAnchorElement", "hash"),
    ("HTMLAnchorElement", "text"),
    ("HTMLAreaElement", "origin"),
    ("HTMLAreaElement", "protocol"),
    ("HTMLAreaElement", "username"),
    ("HTMLAreaElement", "password"),
    ("HTMLAreaElement", "host"),
    ("HTMLAreaElement", "hostname"),
    ("HTMLAreaElement", "port"),
    ("HTMLAreaElement", "pathname"),
    ("HTMLAreaElement", "search"),
    ("HTMLAreaElement", "hash"),
];

/// Attributes of event interfaces whose init dictionary member has no default,
/// so the glue says what an omitted one reads as: `H.<Interface>_<member>`.
const JS_FIELDS: &[(&str, &str)] = &[
    ("PointerEvent", "tiltX"),
    ("PointerEvent", "tiltY"),
    ("PointerEvent", "altitudeAngle"),
    ("PointerEvent", "azimuthAngle"),
];

/// In a table's member list: every attribute and operation in the IDL,
/// and the constructor. For event interfaces, whose members need no Rust.
const ALL_MEMBERS: &str = "*";

/// Operations for the glue that aren't IDL members: the name its `OP_<name>`
/// constant takes, and the call on the host.
const HOST_OPS: &[(&str, &str)] = &[
    ("setReadyState", "h.set_ready_state(a.uint(0)?)"),
    ("observerNew", "h.observer_new()"),
    (
        "observe",
        "h.observe(a.uint(0)?, a.id(1)?, a.uint(2)?, a.string(3)?)",
    ),
    ("observerDisconnect", "h.observer_disconnect(a.uint(0)?)"),
    ("observerTake", "h.observer_take(a.uint(0)?)"),
    ("recordsPending", "h.records_pending()"),
    ("layoutQuery", "h.layout_query(a.string(0)?)"),
    ("computedStyle", "h.computed_style(this)"),
    ("serializeXml", "h.serialize_xml(a.id(0)?)"),
    ("nextFrameLoad", "h.next_frame_load()"),
    ("loadFrame", "h.load_frame()"),
    ("isFrameDocument", "h.is_frame_document(this)"),
    (
        "parseFromString",
        "h.parse_from_string(this, a.string(0)?, a.string(1)?)",
    ),
    ("urlPart", "h.url_part(this, a.string(0)?)"),
    ("urlJoin", "h.url_join(this, a.string(0)?)"),
    ("urlFragment", "h.url_fragment(this, a.string(0)?)"),
    ("urlRewrite", "h.url_rewrite(this, a.string(0)?)"),
    ("setDocumentUrl", "h.set_document_url(this, a.string(0)?)"),
    ("inPage", "h.in_page(this)"),
    ("radioCheck", "h.radio_check(this)"),
    ("handlerElement", "h.handler_element()"),
    ("idNames", "h.id_names()"),
    ("labeledControl", "h.labeled_control(this)"),
    ("svgElements", "h.svg_elements()"),
    ("frameAttached", "h.frame_attached(this)"),
    ("htmlList", "h.html_list_of(this, a.uint(0)?)"),
    ("listOwner", "h.list_owner(this)"),
    (
        "sheetInsert",
        "h.sheet_insert(this, a.uint(0)?, a.string(1)?)",
    ),
    ("sheetDelete", "h.sheet_delete(this, a.uint(0)?)"),
    ("iteratorNew", "h.iterator_new(this)"),
    ("iteratorReference", "h.iterator_reference(a.uint(0)?)"),
    ("iteratorBefore", "h.iterator_before(a.uint(0)?)"),
    (
        "iteratorTry",
        "h.iterator_try(a.uint(0)?, a.id(1)?, a.boolean(2)?)",
    ),
    ("iteratorTrialNode", "h.iterator_trial_node(a.uint(0)?)"),
    ("iteratorTrialBefore", "h.iterator_trial_before(a.uint(0)?)"),
    (
        "iteratorSet",
        "h.iterator_set(a.uint(0)?, a.id(1)?, a.boolean(2)?)",
    ),
    (
        "staticRange",
        "h.static_range(a.id(0)?, a.uint(1)?, a.id(2)?, a.uint(3)?)",
    ),
];

/// A member name for an interface's bare `stringifier;`: its `toString`,
/// answered by the host's `stringifier`.
const STRINGIFIER: &str = "stringifier";

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
/// The operations from [`HOST_OPS`] start here.
const OP_HOST: u32 = 2;
/// The first operation number for generated members.
const OP_FIRST: u32 = OP_HOST + HOST_OPS.len() as u32;

/// Members the glue calls itself, for event paths and handler attributes:
/// (interface, member). Each gets a `const OP_<member>` in `bindings.js`.
const GLUE_OPS: &[(&str, &str)] = &[
    ("Node", "parentNode"),
    ("Node", "ownerDocument"),
    ("Element", "localName"),
    ("Element", "getAttribute"),
    ("Element", "getAttributeNames"),
    ("Document", "body"),
    ("Document", "documentElement"),
    ("Document", "getElementById"),
    ("Node", "isConnected"),
    ("Element", "setAttribute"),
    ("Element", "removeAttribute"),
    ("Element", "querySelector"),
    ("Element", "firstElementChild"),
    ("Element", "nextElementSibling"),
    ("HTMLInputElement", "type"),
    ("HTMLInputElement", "checked"),
    ("HTMLInputElement", "set_checked"),
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
    /// Dictionary converters, each once, and the names written so far.
    dicts: String,
    dict_names: BTreeSet<String>,
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

/// `WindowProxy` isn't an interface of its own: WebIDL treats it as `Window`.
fn window_proxy(name: &str) -> &str {
    if name == "WindowProxy" {
        "Window"
    } else {
        name
    }
}

/// Whether `name` is `Event` or inherits from it.
fn is_event(set: &IdlSet, name: &str) -> bool {
    let mut at = name.to_owned();
    // Parent chains are short; the bound stops a cycle.
    for _ in 0..16 {
        if at == "Event" {
            return true;
        }
        match set.interface(&at).and_then(|i| i.parent) {
            Some(p) => at = p,
            None => return false,
        }
    }
    false
}

/// The members of dictionary `name` itself (partials included) in the
/// lexicographic order WebIDL reads them, and what it inherits from. A member
/// that two partials declare keeps the first, by file name.
fn dictionary_parts(set: &IdlSet, name: &str) -> Option<(Vec<DictionaryMember>, Option<String>)> {
    let mut members: Vec<DictionaryMember> = Vec::new();
    let mut parent = None;
    let mut found = false;
    for (_, d) in set.definitions() {
        let DefinitionKind::Dictionary(x) = &d.kind else {
            continue;
        };
        if x.name != name {
            continue;
        }
        found = true;
        if x.inherits.is_some() {
            parent.clone_from(&x.inherits);
        }
        for m in &x.members {
            if !members.iter().any(|o| o.name == m.name) {
                members.push(m.clone());
            }
        }
    }
    members.sort_by(|a, b| a.name.cmp(&b.name));
    found.then_some((members, parent))
}

/// Every member name of dictionary `name`, inherited ones too.
fn dictionary_fields(set: &IdlSet, name: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut at = Some(name.to_owned());
    // Parent chains are short; the bound stops a cycle.
    for _ in 0..16 {
        let Some((members, parent)) = at.as_deref().and_then(|n| dictionary_parts(set, n)) else {
            break;
        };
        out.extend(members.into_iter().map(|m| m.name));
        at = parent;
    }
    out
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
    /// `(boolean or ImportNodeOptions)`: whether the whole subtree is meant.
    DeepFlag,
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
                    let other = window_proxy(other);
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
                let boolean = ts
                    .iter()
                    .any(|b| matches!(&b.kind, TypeKind::Named(n) if n == "boolean"));
                return Some(if boolean { Conv::DeepFlag } else { Conv::Skip });
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
        Conv::DeepFlag => format!("deepFlag({x})"),
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
    /// `Event` or an interface inheriting from it.
    event: bool,
    /// Members of the event's init dictionary, its generated fields.
    fields: BTreeSet<String>,
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
        if self.event
            && self.iface.name != "Event"
            && self.fields.contains(name)
            && !JS_FIELDS.contains(&(self.iface.name.as_str(), name))
        {
            // A nullable member with no default was never written.
            let body = if ty.nullable {
                format!("const v = self(this, {id}).d.{name}; return v === undefined ? null : v;")
            } else {
                format!("return self(this, {id}).d.{name};")
            };
            let _ = writeln!(
                self.out.js,
                "attr({id}, {}, function () {{ {body} }});",
                js_string(name)
            );
            return Ok(());
        }
        // In an interface of the glue's, what the attribute's text answers needs no
        // body there.
        if self.js_impl && self.reflected_attribute(name, ty, readonly, ext_attrs) {
            return Ok(());
        }
        if self.js_impl || JS_MEMBERS.contains(&(self.iface.name.as_str(), name)) {
            return self.js_attribute(name, ty, readonly, ext_attrs);
        }
        if self.reflected_attribute(name, ty, readonly, ext_attrs) {
            return Ok(());
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
            self.note_glue_op(&format!("set_{name}"), op);
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
    fn js_attribute(
        &mut self,
        name: &str,
        ty: &Type,
        readonly: bool,
        ext_attrs: &[ExtAttr],
    ) -> Result<(), GenError> {
        let (id, iface) = (self.id, self.iface.name.clone());
        let _ = write!(
            self.out.js,
            "attr({id}, {}, function () {{ return H.{iface}_{name}(self(this, {id})); }}",
            js_string(name)
        );
        if !readonly {
            let x = self.js_impl_arg(ty, "v")?;
            let _ = write!(
                self.out.js,
                ", function (v) {{ H.set_{iface}_{name}(self(this, {id}), {x}); }}"
            );
        } else if has_ext(ext_attrs, "Replaceable") {
            let _ = write!(
                self.out.js,
                ", function (v) {{ self(this, {id}); replaceable(this === undefined || this === null ? G : this, {}, v); }}",
                js_string(name)
            );
        } else if let Some(fwd) = ext_attrs.iter().find(|a| a.name == "PutForwards") {
            let ExtAttrValue::Ident(target) = &fwd.value else {
                return Err(self.unsupported(name, ty));
            };
            let _ = write!(
                self.out.js,
                ", function (v) {{ H.{iface}_{name}(self(this, {id})).{target} = v; }}"
            );
        }
        self.out.js.push_str(");\n");
        Ok(())
    }

    /// An attribute that `[Reflect]` ties to a content attribute (HTML
    /// §2.6.1), for the types that need no more than the attribute's text.
    /// Written for a listed member with no method on the host, so a new one
    /// costs a table entry. Returns whether it wrote one.
    fn reflected_attribute(
        &mut self,
        name: &str,
        ty: &Type,
        readonly: bool,
        ext_attrs: &[ExtAttr],
    ) -> bool {
        let Some(r) = ext_attrs.iter().find(|a| a.name == "Reflect") else {
            return false;
        };
        if readonly {
            return false;
        }
        let TypeKind::Named(n) = &ty.kind else {
            return false;
        };
        let content = match &r.value {
            ExtAttrValue::String(s) => s.clone(),
            _ => name.to_ascii_lowercase(),
        };
        let content = js_string(&content);
        let id = self.id;
        let (get, set) = match n.as_str() {
            "boolean" if !ty.nullable => ("reflectBool", "setReflectBool"),
            s if is_string(s) && !ty.nullable => ("reflectStr", "setReflectStr"),
            _ => return false,
        };
        let _ = writeln!(
            self.out.js,
            "attr({id}, {}, function () {{ return {get}(self(this, {id}), {content}); }}, function (v) {{ {set}(self(this, {id}), {content}, v); }});",
            js_string(name)
        );
        true
    }

    /// `x` converted for a hand-written member: as for a native one, except
    /// that bound objects stay records and what `conv` leaves alone
    /// (dictionaries, callbacks, `any`) arrives as it came. In an event
    /// interface, where that means the legacy initializers, it is converted as
    /// a dictionary member would be: an unbound interface can only be null.
    fn js_impl_arg(&mut self, ty: &Type, x: &str) -> Result<String, GenError> {
        // A union of a sequence and a string: the glue tells them apart.
        let by_hand = matches!(&ty.kind, TypeKind::Union(ts)
            if ts.iter().any(|t| matches!(t.kind, TypeKind::Sequence(_))));
        if by_hand {
            return Ok(x.to_owned());
        }
        Ok(match conv(self.gen_, self.table, ty, 0) {
            Some(Conv::Obj(id)) if ty.nullable => {
                format!("({x} === null || {x} === undefined ? null : self({x}, {id}))")
            }
            Some(Conv::Obj(id)) => format!("self({x}, {id})"),
            Some(c @ (Conv::Str(_) | Conv::Bool | Conv::Int(_))) => js_arg(&c, ty, x, ""),
            _ if self.event => self.value_conv(ty, x, 0)?,
            _ => x.to_owned(),
        })
    }

    /// Declared parameters and argument expressions of a hand-written
    /// member. An omitted optional argument takes its default, undefined
    /// for a dictionary; the glue reads dictionaries itself.
    fn js_impl_arguments(
        &mut self,
        args: &[Argument],
    ) -> Result<(Vec<String>, Vec<String>), GenError> {
        let mut params = Vec::new();
        let mut exprs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            if a.optional {
                let v = format!("o{i}");
                let x = format!("(argc > {i} ? arguments[{i}] : undefined)");
                let converted = self.js_impl_arg(&a.ty, &v)?;
                let default = js_default(&a.default);
                exprs.push(format!(
                    "((({v}) => {v} === undefined ? {default} : {converted})({x}))"
                ));
            } else {
                let p = format!("a{i}");
                exprs.push(self.js_impl_arg(&a.ty, &p)?);
                params.push(p);
            }
        }
        Ok((params, exprs))
    }

    fn js_operation(
        &mut self,
        name: &str,
        args: &[Argument],
        is_static: bool,
    ) -> Result<(), GenError> {
        let (id, iface) = (self.id, self.iface.name.clone());
        let (params, exprs) = self.js_impl_arguments(args)?;
        let required = params.len();
        let mut body = if is_static {
            "const argc = arguments.length; ".to_owned()
        } else {
            format!("const s = self(this, {id}); const argc = arguments.length; ")
        };
        if required > 0 {
            let _ = write!(body, "need(argc, {required}, {}); ", js_string(name));
        }
        let mut all = if is_static {
            Vec::new()
        } else {
            vec!["s".to_owned()]
        };
        all.extend(exprs);
        let define = if is_static { "staticOp" } else { "op" };
        let quoted = js_string(name);
        let params = params.join(", ");
        let call = format!("{body}return H.{iface}_{name}({});", all.join(", "));
        // A reserved word can name a method, not a function.
        let f = if name == "delete" {
            format!("{{ {quoted}({params}) {{ {call} }} }}[{quoted}]")
        } else {
            format!("function {name}({params}) {{ {call} }}")
        };
        let _ = writeln!(self.out.js, "{define}({id}, {quoted}, {f});");
        Ok(())
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
        if self.js_impl || JS_MEMBERS.contains(&(self.iface.name.as_str(), name)) {
            let is_static = matches!(
                &mm.member.kind,
                MemberKind::Operation {
                    is_static: true,
                    ..
                }
            );
            return self.js_operation(name, args, is_static);
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
        if self.event {
            return self.event_constructor(args);
        }
        if self.js_impl {
            let (params, exprs) = self.js_impl_arguments(args)?;
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

    /// The constructor of an event interface: `(DOMString type, init)`.
    fn event_constructor(&mut self, args: &[Argument]) -> Result<String, GenError> {
        let [_, init] = args else {
            return Err(self.unknown("constructor"));
        };
        let TypeKind::Named(dict) = &init.ty.kind else {
            return Err(self.unsupported("constructor", &init.ty));
        };
        self.dictionary(dict)?;
        let id = self.id;
        let name = &self.iface.name;
        let (params, required, value) = if init.optional {
            ("a0", 1, "(argc > 1 ? arguments[1] : undefined)")
        } else {
            ("a0, a1", 2, "a1")
        };
        Ok(format!(
            "function {name}({params}) {{ if (new.target === undefined) throw new TE('Constructor requires new'); const argc = arguments.length; need(argc, {required}, {}); return newEvent({id}, new.target, str(a0), D_{dict}({value})); }}",
            js_string(name)
        ))
    }

    /// Writes `D_<name>(value, out)` once, and those it needs: WebIDL §3.2.18.
    /// With `out` left undefined, `value` is a whole dictionary to convert;
    /// from a derived dictionary, `value` is already an object.
    fn dictionary(&mut self, name: &str) -> Result<(), GenError> {
        if !self.out.dict_names.insert(name.to_owned()) {
            return Ok(());
        }
        let Some((members, parent)) = dictionary_parts(self.gen_.set, name) else {
            return Err(GenError::UnknownInterface(name.to_owned()));
        };
        let mut body = format!(
            "function D_{name}(v, d) {{\n  if (d === undefined) {{ v = dictionary(v, {}); d = create(null); }}\n",
            js_string(name)
        );
        if let Some(p) = &parent {
            self.dictionary(p)?;
            let _ = writeln!(body, "  D_{p}(v, d);");
        }
        body.push_str("  let x;\n");
        for m in &members {
            let k = &m.name;
            let _ = write!(body, "  x = v.{k}; ");
            if m.required {
                let _ = write!(
                    body,
                    "if (x === undefined) throw new TE({}); ",
                    js_string(&format!("{name}: {k} is required"))
                );
            }
            let c = self.value_conv(&m.ty, "x", 0)?;
            let default = match &m.default {
                Some(DefaultValue::EmptySequence) => "realArray(list())".to_owned(),
                Some(DefaultValue::EmptyDictionary) => match &m.ty.kind {
                    TypeKind::Named(n) => format!("D_{n}(undefined)"),
                    _ => return Err(self.unsupported(k, &m.ty)),
                },
                d => js_default(d),
            };
            let _ = writeln!(body, "d.{k} = x === undefined ? {default} : {c};");
        }
        body.push_str("  return d;\n}\n");
        self.out.dicts.push_str(&body);
        Ok(())
    }

    /// JS that converts `x`, already known not to be undefined at the top
    /// level, to IDL type `t`, kept as a JavaScript value: WebIDL §3.2 for
    /// what stays in the glue. A value of an interface with no binding can
    /// only be null, since no script can have one.
    fn value_conv(&mut self, t: &Type, x: &str, depth: u32) -> Result<String, GenError> {
        if depth > 8 {
            return Err(self.unsupported(x, t));
        }
        let c = self.plain_value_conv(t, x, depth)?;
        Ok(if t.nullable {
            format!("({x} === null || {x} === undefined ? null : {c})")
        } else {
            c
        })
    }

    fn plain_value_conv(&mut self, t: &Type, x: &str, depth: u32) -> Result<String, GenError> {
        let n = match &t.kind {
            TypeKind::Named(n) => n.as_str(),
            TypeKind::Sequence(e) | TypeKind::FrozenArray(e) => {
                let c = self.value_conv(e, "e", depth + 1)?;
                return Ok(format!("seq({x}, (e) => {c})"));
            }
            TypeKind::Promise(_) => return Ok(format!("toPromise({x})")),
            TypeKind::Union(ts) => {
                // Unions here are one bound interface among unbound ones.
                let found = ts.iter().find_map(|b| match &b.kind {
                    TypeKind::Named(n) => {
                        bound_ancestor(self.gen_.set, self.table, window_proxy(n))
                    }
                    _ => None,
                });
                return Ok(match found {
                    Some(id) => format!("inst({x}, {id})"),
                    None => format!("unbound({x})"),
                });
            }
            _ => return Err(self.unsupported(x, t)),
        };
        Ok(match n {
            "any" => x.to_owned(),
            "object" => format!("objectOf({x})"),
            "boolean" => format!("!!{x}"),
            "byte" => format!("(({x} << 24) >> 24)"),
            "octet" => format!("({x} & 255)"),
            "short" => format!("(({x} << 16) >> 16)"),
            "unsigned short" => format!("(({x} >>> 0) & 65535)"),
            "long" => format!("({x} | 0)"),
            "unsigned long" => format!("({x} >>> 0)"),
            "double" => format!("dbl({x})"),
            "float" => format!("flt({x})"),
            "unrestricted double" => format!("+{x}"),
            "DOMString" | "CSSOMString" => format!("`${{{x}}}`"),
            "USVString" | "ByteString" => format!("str({x})"),
            other => {
                let other = window_proxy(other);
                if let Some(id) = bound_ancestor(self.gen_.set, self.table, other) {
                    return Ok(format!("inst({x}, {id})"));
                }
                if let Some(target) = self.gen_.set.typedef(other).cloned() {
                    return self.value_conv(&target, x, depth + 1);
                }
                match self.gen_.names.get(other) {
                    Some(NameKind::Dictionary) => {
                        self.dictionary(other)?;
                        format!("D_{other}({x})")
                    }
                    Some(NameKind::Enum) => {
                        let values = self.enum_values(other);
                        format!("oneOf(`${{{x}}}`, [{}])", values.join(", "))
                    }
                    _ => format!("unbound({x})"),
                }
            }
        })
    }

    fn enum_values(&self, name: &str) -> Vec<String> {
        self.gen_
            .set
            .definitions()
            .find_map(|(_, d)| match &d.kind {
                DefinitionKind::Enum { name: n, values } if n == name => {
                    Some(values.iter().map(|v| js_string(v)).collect())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The name of the init dictionary of this event interface, if any.
    fn init_dictionary(&self) -> Option<String> {
        self.iface
            .members
            .iter()
            .find_map(|m| match &m.member.kind {
                MemberKind::Constructor { args } => match args.last().map(|a| &a.ty.kind) {
                    Some(TypeKind::Named(n)) => Some(n.clone()),
                    _ => None,
                },
                _ => None,
            })
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

    /// What [`ALL_MEMBERS`] stands for: each attribute and operation once
    /// (two partials can declare the same one), no static ones, no anonymous
    /// ones, and the constructor if there is one.
    fn all_members(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for m in &self.iface.members {
            let name = match &m.member.kind {
                MemberKind::Attribute {
                    name,
                    is_static: false,
                    ty,
                    ..
                } if !matches!(&ty.kind, TypeKind::Named(n)
                        if EVENT_HANDLER_TYPES.contains(&n.as_str())) =>
                {
                    name.clone()
                }
                MemberKind::Operation {
                    name: Some(name),
                    is_static: false,
                    special: None,
                    ..
                } => name.clone(),
                MemberKind::Constructor { .. } => "constructor".to_owned(),
                _ => continue,
            };
            if !out.contains(&name) {
                out.push(name);
            }
        }
        out
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
        let everything = spec.members == [ALL_MEMBERS];
        let expanded: Vec<String> = if everything {
            self.all_members()
        } else {
            spec.members.iter().map(|m| (*m).to_owned()).collect()
        };
        let members: Vec<&str> = expanded.iter().map(String::as_str).collect();
        let dict = self.init_dictionary().filter(|_| self.event);
        if let Some(d) = &dict {
            self.fields = dictionary_fields(self.gen_.set, d);
        }
        let ctor = self.constructor(&members)?;
        let _ = writeln!(
            self.out.js,
            "iface({id}, {}, {parent}, {ctor});",
            js_string(spec.name)
        );
        if self.event {
            match &dict {
                Some(d) => {
                    let _ = writeln!(self.out.js, "eventInits[{id}] = D_{d};");
                }
                None => {
                    let _ = writeln!(self.out.js, "eventInits[{id}] = eventInits[{parent}];");
                }
            }
        }
        for mm in &self.iface.members.clone() {
            if let MemberKind::Const { name, value, .. } = &mm.member.kind {
                let _ = writeln!(self.out.js, "konst({id}, {}, {value});", js_string(name));
            }
        }
        let mut seen = BTreeSet::new();
        for &name in &members {
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
                    | STRINGIFIER
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
        if members.contains(&"iterable") {
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
        if let Some(s) = stringifies.filter(|s| members.contains(&s.as_str())) {
            let _ = writeln!(self.out.js, "stringifier({id}, {});", js_string(&s));
        }
        if members.contains(&STRINGIFIER) {
            if !self
                .iface
                .members
                .iter()
                .any(|m| matches!(m.member.kind, MemberKind::Stringifier))
            {
                return Err(self.unknown(STRINGIFIER));
            }
            if self.js_impl {
                let _ = writeln!(
                    self.out.js,
                    "op({id}, \"toString\", function toString() {{ return H.{}_toString(self(this, {id})); }});",
                    self.iface.name
                );
                self.event_handlers();
                return Ok(());
            }
            let call = self.trait_call(STRINGIFIER, &[], true);
            let op = self.out.op(&call);
            let _ = writeln!(
                self.out.js,
                "op({id}, \"toString\", function toString() {{ const s = self(this, {id}); return call({op}, s.g, s.h); }});"
            );
        }
        if let Some(x) = self.exotic(&members)? {
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
        dicts: String::new(),
        dict_names: BTreeSet::new(),
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
            js_impl: JS_INTERFACES.contains(&spec.name) || is_event(set, spec.name),
            event: is_event(set, spec.name),
            fields: BTreeSet::new(),
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
    for name in JS_INTERFACES.iter().chain(&[
        "Node",
        "Element",
        "HTMLElement",
        "Document",
        "HTMLIFrameElement",
        "HTMLBodyElement",
        "HTMLFrameSetElement",
        "HTMLFormElement",
        "SVGElement",
        "Attr",
        "Range",
        "HTMLCollection",
        "HTMLFormControlsCollection",
        "HTMLOptGroupElement",
        "HTMLImageElement",
        "HTMLAnchorElement",
        "HTMLAreaElement",
        "HTMLStyleElement",
    ]) {
        if let Some(id) = bound_id(table, name) {
            let _ = writeln!(js, "const I_{name} = {id};");
        }
    }
    for spec in table {
        if is_event(set, spec.name)
            && spec.name != "Event"
            && let Some(id) = bound_id(table, spec.name)
        {
            let _ = writeln!(js, "const I_{} = {id};", spec.name);
        }
    }
    js.push_str(&out.dicts);
    js.push_str(&out.js);

    for (i, (name, _)) in HOST_OPS.iter().enumerate() {
        let _ = writeln!(js, "const OP_{name} = {};", OP_HOST + i as u32);
    }
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
    for (i, (_, call)) in HOST_OPS.iter().enumerate() {
        let _ = writeln!(
            rust,
            "        {} => {call}.map(Ret::from),",
            OP_HOST + i as u32
        );
    }
    rust.push_str(&out.arms);
    rust.push_str(
        "        _ => Err(BindingError::TypeError(\"unknown operation\".to_owned())),\n    }\n}\n",
    );
    Ok(Bindings { js, rust })
}

/// After a writable attribute's name in a table: expose the getter only, for
/// a member whose setter isn't implemented yet.
const GETTER_ONLY: &str = ":getter";

/// An event interface, whole: see [`JS_INTERFACES`].
const fn event(name: &'static str) -> BindInterface {
    BindInterface {
        name,
        members: &[ALL_MEMBERS],
    }
}

/// An interface bound for its name and brand alone.
const fn element(name: &'static str) -> BindInterface {
    BindInterface { name, members: &[] }
}

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
            "timeStamp",
            "initEvent",
        ],
    },
    event("CustomEvent"),
    event("ErrorEvent"),
    event("UIEvent"),
    event("FocusEvent"),
    event("MouseEvent"),
    event("WheelEvent"),
    event("PointerEvent"),
    event("DragEvent"),
    event("KeyboardEvent"),
    event("CompositionEvent"),
    event("InputEvent"),
    event("TextEvent"),
    event("TouchEvent"),
    event("HashChangeEvent"),
    event("PopStateEvent"),
    event("PageTransitionEvent"),
    event("BeforeUnloadEvent"),
    event("PromiseRejectionEvent"),
    event("MessageEvent"),
    event("StorageEvent"),
    event("SubmitEvent"),
    event("ToggleEvent"),
    event("CommandEvent"),
    event("DeviceMotionEvent"),
    event("DeviceOrientationEvent"),
    BindInterface {
        name: "Window",
        members: &[
            "devicePixelRatio",
            "event",
            "top",
            "parent",
            "frames",
            "length",
            "closed",
            "frameElement",
            "opener",
            "location",
            "history",
            "navigator",
            "clientInformation",
            "customElements",
            "atob",
            "btoa",
            "structuredClone",
        ],
    },
    BindInterface {
        name: "Location",
        members: &[
            "href", "origin", "protocol", "host", "hostname", "port", "pathname", "search", "hash",
            "assign", "replace", "reload",
        ],
    },
    BindInterface {
        name: "History",
        members: &[
            "length",
            "scrollRestoration",
            "state",
            "go",
            "back",
            "forward",
            "pushState",
            "replaceState",
        ],
    },
    // The sandbox's object has the members that exist so far.
    BindInterface {
        name: "Performance",
        members: &[],
    },
    BindInterface {
        name: "AbortController",
        members: &["constructor", "signal", "abort"],
    },
    BindInterface {
        name: "AbortSignal",
        members: &[
            "abort",
            "timeout",
            "any",
            "aborted",
            "reason",
            "throwIfAborted",
        ],
    },
    BindInterface {
        name: "Node",
        members: &[
            "nodeType",
            "nodeName",
            "baseURI",
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
            "lookupPrefix",
            "lookupNamespaceURI",
            "isDefaultNamespace",
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
            "getAttributeNode",
            "getAttributeNodeNS",
            "setAttributeNode",
            "setAttributeNodeNS",
            "removeAttributeNode",
            "getElementsByTagNameNS",
            "closest",
            "matches",
            "webkitMatchesSelector",
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
        members: &[
            "title",
            "lang",
            "dir",
            "dataset",
            "style",
            "click",
            "innerText",
            "focus",
            "blur",
            "tabIndex",
        ],
    },
    BindInterface {
        name: "SVGElement",
        members: &["style", "focus", "blur", "tabIndex"],
    },
    BindInterface {
        name: "Document",
        members: &[
            "constructor",
            "implementation",
            "URL",
            "documentURI",
            "charset",
            "inputEncoding",
            "doctype",
            "createCDATASection",
            "scripts",
            "createAttribute",
            "rootElement",
            "createRange",
            "createAttributeNS",
            "createProcessingInstruction",
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
            "getElementsByTagNameNS",
            "getElementsByClassName",
            "querySelector",
            "querySelectorAll",
            "createElement",
            "createElementNS",
            "createDocumentFragment",
            "createTextNode",
            "createComment",
            "createEvent",
            "cookie",
            "styleSheets",
            "createNodeIterator",
            "createTreeWalker",
            "forms",
            "images",
            "embeds",
            "plugins",
            "links",
            "anchors",
            "activeElement",
            "hasFocus",
            "importNode",
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
        name: "XMLDocument",
        members: &[],
    },
    BindInterface {
        name: "DOMImplementation",
        members: &[
            "createDocumentType",
            "createDocument",
            "createHTMLDocument",
            "hasFeature",
        ],
    },
    BindInterface {
        name: "DocumentType",
        members: &[
            "name",
            "publicId",
            "systemId",
            "before",
            "after",
            "replaceWith",
            "remove",
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
    // Shadow trees are out of scope, but libraries test `instanceof ShadowRoot`.
    BindInterface {
        name: "ShadowRoot",
        members: &[],
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
        name: "CDATASection",
        members: &[],
    },
    BindInterface {
        name: "Comment",
        members: &["constructor"],
    },
    BindInterface {
        name: "ProcessingInstruction",
        members: &["target"],
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
            "setNamedItem",
            "setNamedItemNS",
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
    BindInterface {
        name: "CSSRule",
        members: &["cssText", "parentRule", "parentStyleSheet", "type"],
    },
    BindInterface {
        name: "CSSGroupingRule",
        members: &[],
    },
    BindInterface {
        name: "CSSStyleRule",
        members: &["selectorText"],
    },
    BindInterface {
        name: "StyleSheet",
        members: &["type", "href", "ownerNode", "parentStyleSheet", "title"],
    },
    BindInterface {
        name: "CSSStyleSheet",
        members: &[
            "ownerRule",
            "cssRules",
            "rules",
            "insertRule",
            "deleteRule",
            "addRule",
            "removeRule",
        ],
    },
    // Their indexed properties are in `glue_sheet.js`, since they have no handle.
    BindInterface {
        name: "StyleSheetList",
        members: &["length"],
    },
    BindInterface {
        name: "CSSRuleList",
        members: &["length"],
    },
    BindInterface {
        name: "MutationObserver",
        members: &["constructor", "observe", "disconnect", "takeRecords"],
    },
    BindInterface {
        name: "MutationRecord",
        members: &[
            "type",
            "target",
            "addedNodes",
            "removedNodes",
            "previousSibling",
            "nextSibling",
            "attributeName",
            "attributeNamespace",
            "oldValue",
        ],
    },
    BindInterface {
        name: "HTMLTemplateElement",
        members: &["content"],
    },
    BindInterface {
        name: "HTMLAnchorElement",
        members: &[
            "href", "rel", "target", "download", "hreflang", "type", "ping", "text", "origin",
            "protocol", "username", "password", "host", "hostname", "port", "pathname", "search",
            "hash",
        ],
    },
    BindInterface {
        name: "HTMLImageElement",
        members: &["src", "alt"],
    },
    BindInterface {
        name: "HTMLLinkElement",
        members: &["href", "rel", "sheet"],
    },
    BindInterface {
        name: "HTMLScriptElement",
        members: &["src", "type"],
    },
    BindInterface {
        name: "HTMLInputElement",
        members: &[
            "value",
            "type",
            "name",
            "checked",
            "defaultChecked",
            "defaultValue",
            "disabled",
            "form",
            "indeterminate",
        ],
    },
    BindInterface {
        name: "DOMParser",
        members: &["constructor", "parseFromString"],
    },
    BindInterface {
        name: "XMLSerializer",
        members: &["constructor", "serializeToString"],
    },
    BindInterface {
        name: "AbstractRange",
        members: &[
            "startContainer",
            "startOffset",
            "endContainer",
            "endOffset",
            "collapsed",
        ],
    },
    BindInterface {
        name: "StaticRange",
        members: &["constructor"],
    },
    BindInterface {
        name: "Range",
        members: &[
            "constructor",
            "commonAncestorContainer",
            "setStart",
            "setEnd",
            "setStartBefore",
            "setStartAfter",
            "setEndBefore",
            "setEndAfter",
            "collapse",
            "selectNode",
            "selectNodeContents",
            "compareBoundaryPoints",
            "deleteContents",
            "extractContents",
            "cloneContents",
            "insertNode",
            "surroundContents",
            "cloneRange",
            "detach",
            "isPointInRange",
            "comparePoint",
            "intersectsNode",
            "createContextualFragment",
            STRINGIFIER,
        ],
    },
    // Layout queries, in `glue_layout.js`.
    BindInterface {
        name: "DOMRectReadOnly",
        members: &[
            "constructor",
            "x",
            "y",
            "width",
            "height",
            "top",
            "right",
            "bottom",
            "left",
            "toJSON",
        ],
    },
    BindInterface {
        name: "DOMRect",
        members: &["constructor", "x", "y", "width", "height"],
    },
    BindInterface {
        name: "DOMRectList",
        members: &["length"],
    },
    // HTML §3.2.2: the interface of every element in the spec's index, so
    // `instanceof` and constructor names are right; their members come as
    // pages need them.
    BindInterface {
        name: "HTMLAreaElement",
        members: &[
            "href", "rel", "target", "download", "alt", "coords", "shape", "ping", "origin",
            "protocol", "username", "password", "host", "hostname", "port", "pathname", "search",
            "hash",
        ],
    },
    element("HTMLBaseElement"),
    element("HTMLBodyElement"),
    element("HTMLBRElement"),
    BindInterface {
        name: "HTMLButtonElement",
        members: &["disabled", "form", "name", "type", "value"],
    },
    element("HTMLCanvasElement"),
    element("HTMLDataElement"),
    element("HTMLDataListElement"),
    BindInterface {
        name: "HTMLDetailsElement",
        members: &["name", "open"],
    },
    element("HTMLDialogElement"),
    element("HTMLDirectoryElement"),
    element("HTMLDivElement"),
    element("HTMLDListElement"),
    element("HTMLEmbedElement"),
    BindInterface {
        name: "HTMLFieldSetElement",
        members: &["disabled", "form", "name"],
    },
    element("HTMLFontElement"),
    BindInterface {
        name: "HTMLFormElement",
        members: &[
            "name",
            "noValidate",
            "target",
            "rel",
            "acceptCharset",
            "action",
            "method",
            "enctype",
            "encoding",
            "elements",
            "length",
            "submit",
            "requestSubmit",
            "reset",
        ],
    },
    element("HTMLFrameElement"),
    element("HTMLFrameSetElement"),
    element("HTMLHeadElement"),
    element("HTMLHeadingElement"),
    element("HTMLHRElement"),
    element("HTMLHtmlElement"),
    BindInterface {
        name: "HTMLIFrameElement",
        members: &["src", "srcdoc", "name", "contentDocument"],
    },
    BindInterface {
        name: "HTMLLabelElement",
        members: &["form", "htmlFor", "control"],
    },
    element("HTMLLegendElement"),
    element("HTMLLIElement"),
    element("HTMLMapElement"),
    element("HTMLMarqueeElement"),
    element("HTMLMediaElement"),
    element("HTMLAudioElement"),
    element("HTMLVideoElement"),
    element("HTMLMenuElement"),
    element("HTMLMetaElement"),
    element("HTMLMeterElement"),
    element("HTMLModElement"),
    element("HTMLObjectElement"),
    element("HTMLOListElement"),
    element("HTMLOptGroupElement"),
    BindInterface {
        name: "HTMLOptionElement",
        members: &[
            "disabled",
            "form",
            "label",
            "defaultSelected",
            "selected",
            "value",
            "text",
            "index",
        ],
    },
    BindInterface {
        name: "HTMLOutputElement",
        members: &["form", "name"],
    },
    element("HTMLParagraphElement"),
    element("HTMLParamElement"),
    element("HTMLPictureElement"),
    element("HTMLPreElement"),
    element("HTMLProgressElement"),
    element("HTMLQuoteElement"),
    BindInterface {
        name: "HTMLSelectElement",
        members: &[
            "disabled",
            "form",
            "multiple",
            "name",
            "required",
            "size",
            "type",
            "options",
            "length",
            "add",
            "selectedOptions",
            "selectedIndex",
            "value",
        ],
    },
    element("HTMLSelectedContentElement"),
    element("HTMLSlotElement"),
    element("HTMLSourceElement"),
    element("HTMLSpanElement"),
    BindInterface {
        name: "HTMLStyleElement",
        members: &["media", "type", "sheet"],
    },
    element("HTMLTableCaptionElement"),
    BindInterface {
        name: "HTMLTableCellElement",
        members: &[
            "colSpan",
            "rowSpan",
            "headers",
            "cellIndex",
            "scope",
            "abbr",
        ],
    },
    element("HTMLTableColElement"),
    BindInterface {
        name: "HTMLTableElement",
        members: &[
            "caption",
            "createCaption",
            "deleteCaption",
            "tHead",
            "createTHead",
            "deleteTHead",
            "tFoot",
            "createTFoot",
            "deleteTFoot",
            "tBodies",
            "createTBody",
            "rows",
            "insertRow",
            "deleteRow",
        ],
    },
    BindInterface {
        name: "HTMLTableRowElement",
        members: &[
            "rowIndex",
            "sectionRowIndex",
            "cells",
            "insertCell",
            "deleteCell",
        ],
    },
    BindInterface {
        name: "HTMLTableSectionElement",
        members: &["rows", "insertRow", "deleteRow"],
    },
    BindInterface {
        name: "HTMLTextAreaElement",
        members: &[
            "disabled",
            "form",
            "name",
            "placeholder",
            "readOnly",
            "required",
            "value",
            "defaultValue",
        ],
    },
    element("HTMLTimeElement"),
    element("HTMLTitleElement"),
    element("HTMLTrackElement"),
    element("HTMLUListElement"),
    element("HTMLUnknownElement"),
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
    BindInterface {
        name: "URL",
        members: &[
            "constructor",
            "parse",
            "canParse",
            "href",
            "origin",
            "protocol",
            "username",
            "password",
            "host",
            "hostname",
            "port",
            "pathname",
            "search",
            "searchParams",
            "hash",
            "toJSON",
        ],
    },
    BindInterface {
        name: "URLSearchParams",
        members: &[
            "constructor",
            "size",
            "append",
            "delete",
            "get",
            "getAll",
            "has",
            "set",
            "sort",
            STRINGIFIER,
        ],
    },
    BindInterface {
        name: "TextEncoder",
        members: &["constructor", "encoding", "encode", "encodeInto"],
    },
    BindInterface {
        name: "TextDecoder",
        members: &["constructor", "encoding", "fatal", "ignoreBOM", "decode"],
    },
    BindInterface {
        name: "Navigator",
        members: &[
            "appCodeName",
            "appName",
            "appVersion",
            "platform",
            "product",
            "productSub",
            "userAgent",
            "vendor",
            "vendorSub",
            "language",
            "languages",
            "onLine",
            "cookieEnabled",
            "plugins",
            "mimeTypes",
            "javaEnabled",
            "pdfViewerEnabled",
            "hardwareConcurrency",
            "maxTouchPoints",
        ],
    },
    // Always empty. Their getters are the glue's, or `exotic` would ask Rust.
    BindInterface {
        name: "PluginArray",
        members: &["length", "refresh"],
    },
    BindInterface {
        name: "MimeTypeArray",
        members: &["length"],
    },
    // A registry and no more: nothing is upgraded (`glue_platform.js`).
    BindInterface {
        name: "CustomElementRegistry",
        members: &["define", "get", "getName", "whenDefined", "upgrade"],
    },
    // Their indexed and named properties are `HTMLCollection`'s: `glue_html.js`.
    BindInterface {
        name: "HTMLOptionsCollection",
        members: &["length", "add", "remove", "selectedIndex"],
    },
    BindInterface {
        name: "HTMLFormControlsCollection",
        members: &[],
    },
    BindInterface {
        name: "NodeIterator",
        members: &[
            "root",
            "referenceNode",
            "pointerBeforeReferenceNode",
            "whatToShow",
            "filter",
            "nextNode",
            "previousNode",
            "detach",
        ],
    },
    BindInterface {
        name: "TreeWalker",
        members: &[
            "root",
            "whatToShow",
            "filter",
            "currentNode",
            "parentNode",
            "firstChild",
            "lastChild",
            "previousSibling",
            "nextSibling",
            "previousNode",
            "nextNode",
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
            b.rust
                .contains(&format!("{OP_FIRST} => idl::A::y(h, this).map(Ret::from),")),
            "{}",
            b.rust
        );
        assert!(
            b.rust.contains(&format!(
                "{} => idl::A::z(h, this, a.string(0)?, a.boolean(1)?).map(Ret::from),",
                OP_FIRST + 1
            )),
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
