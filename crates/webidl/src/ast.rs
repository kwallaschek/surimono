//! The syntax tree for the subset of WebIDL that specs use, and a printer
//! back to normalized IDL (one definition per line group, one member per line).
//!
//! Identifiers are stored with WebIDL's leading-underscore escape removed
//! (`_interface` becomes `interface`), as the spec requires.

use std::fmt::{self, Display, Formatter, Write as _};

/// A top-level definition with its extended attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct Definition {
    /// Extended attributes before the definition, such as `[Exposed=Window]`.
    pub ext_attrs: Vec<ExtAttr>,
    /// What kind of definition this is.
    pub kind: DefinitionKind,
    /// 1-based line of the definition's first token (after its extended attributes).
    pub line: u32,
}

/// The kinds of top-level definition.
#[derive(Debug, Clone, PartialEq)]
pub enum DefinitionKind {
    /// `interface`, `interface mixin` or `callback interface`, possibly `partial`.
    Interface(Interface),
    /// `namespace`, possibly `partial`.
    Namespace(Namespace),
    /// `dictionary`, possibly `partial`.
    Dictionary(Dictionary),
    /// `enum Name { "a", "b" };`
    Enum {
        /// Enum name.
        name: String,
        /// Allowed string values, in order.
        values: Vec<String>,
    },
    /// `typedef Type Name;`
    Typedef {
        /// The aliased type.
        ty: Type,
        /// The new name.
        name: String,
    },
    /// `callback Name = Return (args);`
    Callback {
        /// Callback name.
        name: String,
        /// Return type.
        return_type: Type,
        /// Arguments.
        args: Vec<Argument>,
    },
    /// `Target includes Mixin;`
    Includes {
        /// The interface receiving the mixin.
        target: String,
        /// The interface mixin.
        mixin: String,
    },
}

/// Which flavour of interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceKind {
    /// `interface`
    Interface,
    /// `interface mixin`
    Mixin,
    /// `callback interface`
    Callback,
}

/// An interface, interface mixin or callback interface.
#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
    /// Which flavour.
    pub kind: InterfaceKind,
    /// `partial interface …`
    pub partial: bool,
    /// Interface name.
    pub name: String,
    /// Inherited interface, if any.
    pub inherits: Option<String>,
    /// Members in source order.
    pub members: Vec<Member>,
}

/// A namespace.
#[derive(Debug, Clone, PartialEq)]
pub struct Namespace {
    /// `partial namespace …`
    pub partial: bool,
    /// Namespace name.
    pub name: String,
    /// Members in source order.
    pub members: Vec<Member>,
}

/// A dictionary.
#[derive(Debug, Clone, PartialEq)]
pub struct Dictionary {
    /// `partial dictionary …`
    pub partial: bool,
    /// Dictionary name.
    pub name: String,
    /// Inherited dictionary, if any.
    pub inherits: Option<String>,
    /// Members in source order.
    pub members: Vec<DictionaryMember>,
}

/// A dictionary member.
#[derive(Debug, Clone, PartialEq)]
pub struct DictionaryMember {
    /// Extended attributes on the member.
    pub ext_attrs: Vec<ExtAttr>,
    /// `required`
    pub required: bool,
    /// Member type.
    pub ty: Type,
    /// Member name.
    pub name: String,
    /// Default value, if any.
    pub default: Option<DefaultValue>,
}

/// A member of an interface, mixin, callback interface or namespace.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    /// Extended attributes on the member.
    pub ext_attrs: Vec<ExtAttr>,
    /// What kind of member.
    pub kind: MemberKind,
}

/// `getter`, `setter` or `deleter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Special {
    /// `getter`
    Getter,
    /// `setter`
    Setter,
    /// `deleter`
    Deleter,
}

/// The kinds of member.
#[derive(Debug, Clone, PartialEq)]
pub enum MemberKind {
    /// `const Type NAME = value;`
    Const {
        /// Constant type.
        ty: Type,
        /// Constant name.
        name: String,
        /// Value.
        value: ConstValue,
    },
    /// An attribute.
    Attribute {
        /// Attribute name.
        name: String,
        /// Attribute type.
        ty: Type,
        /// `readonly`
        readonly: bool,
        /// `inherit`
        inherit: bool,
        /// `static`
        is_static: bool,
        /// `stringifier attribute …`
        stringifier: bool,
    },
    /// A regular, static or special operation.
    Operation {
        /// Name; `None` for an anonymous special operation.
        name: Option<String>,
        /// Return type.
        return_type: Type,
        /// Arguments.
        args: Vec<Argument>,
        /// `getter`, `setter` or `deleter`.
        special: Option<Special>,
        /// `static`
        is_static: bool,
        /// `stringifier Type name(…);`
        stringifier: bool,
    },
    /// `constructor(args);`
    Constructor {
        /// Arguments.
        args: Vec<Argument>,
    },
    /// A bare `stringifier;`.
    Stringifier,
    /// `iterable<V>`, `iterable<K, V>` or `async iterable<…>(args)`.
    Iterable {
        /// `async`
        is_async: bool,
        /// Key type for pair iterators.
        key: Option<Type>,
        /// Value type.
        value: Type,
        /// Arguments of an async iterable, if given.
        args: Option<Vec<Argument>>,
    },
    /// `maplike<K, V>`
    Maplike {
        /// `readonly`
        readonly: bool,
        /// Key type.
        key: Type,
        /// Value type.
        value: Type,
    },
    /// `setlike<V>`
    Setlike {
        /// `readonly`
        readonly: bool,
        /// Value type.
        value: Type,
    },
}

/// An operation, constructor or callback argument.
#[derive(Debug, Clone, PartialEq)]
pub struct Argument {
    /// Extended attributes on the argument.
    pub ext_attrs: Vec<ExtAttr>,
    /// `optional`
    pub optional: bool,
    /// Argument type.
    pub ty: Type,
    /// `...`
    pub variadic: bool,
    /// Argument name.
    pub name: String,
    /// Default value, if any.
    pub default: Option<DefaultValue>,
}

/// A type, with its own extended attributes and nullability.
#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    /// Extended attributes on the type, such as `[LegacyNullToEmptyString]`.
    pub ext_attrs: Vec<ExtAttr>,
    /// The type itself.
    pub kind: TypeKind,
    /// `T?`
    pub nullable: bool,
}

/// The kinds of type.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    /// A built-in type by its canonical spelling (`unsigned long long`,
    /// `unrestricted double`, `DOMString`, `any`, `undefined`, …) or the name
    /// of an interface, dictionary, enum, typedef or callback.
    Named(String),
    /// `sequence<T>`
    Sequence(Box<Type>),
    /// `FrozenArray<T>`
    FrozenArray(Box<Type>),
    /// `ObservableArray<T>`
    ObservableArray(Box<Type>),
    /// `Promise<T>`
    Promise(Box<Type>),
    /// `record<K, V>`
    Record(Box<Type>, Box<Type>),
    /// `(A or B or …)`
    Union(Vec<Type>),
}

impl Type {
    /// A plain named type with no extended attributes.
    pub fn named(name: &str) -> Self {
        Type {
            ext_attrs: Vec::new(),
            kind: TypeKind::Named(name.to_owned()),
            nullable: false,
        }
    }
}

/// A constant value.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    /// `true` or `false`
    Bool(bool),
    /// An integer literal as written (decimal, `0x` hex or `0` octal).
    Integer(String),
    /// A decimal literal as written, or `Infinity`, `-Infinity`, `NaN`.
    Float(String),
}

/// A default value for an optional argument or dictionary member.
#[derive(Debug, Clone, PartialEq)]
pub enum DefaultValue {
    /// A constant.
    Const(ConstValue),
    /// A string literal.
    String(String),
    /// `[]`
    EmptySequence,
    /// `{}`
    EmptyDictionary,
    /// `null`
    Null,
    /// `undefined`
    Undefined,
}

/// One extended attribute, such as `Exposed=Window` or `LegacyFactoryFunction=Image(…)`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtAttr {
    /// Attribute name.
    pub name: String,
    /// Its right-hand side and arguments.
    pub value: ExtAttrValue,
}

/// The forms an extended attribute takes.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtAttrValue {
    /// `[Name]`
    None,
    /// `[Name(args)]`
    Args(Vec<Argument>),
    /// `[Name=Ident]`
    Ident(String),
    /// `[Name=*]`
    Wildcard,
    /// `[Name="string"]`
    String(String),
    /// `[Name=1]` or `[Name=1.5]`
    Number(String),
    /// `[Name=(A, B)]` (identifiers, strings or numbers, as written)
    List(Vec<String>),
    /// `[Name=Ident(args)]`
    NamedArgs(String, Vec<Argument>),
}

impl ExtAttr {
    /// The identifier on the right-hand side, if it is `Name=Ident`.
    pub fn ident(&self) -> Option<&str> {
        match &self.value {
            ExtAttrValue::Ident(s) => Some(s),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Printer: normalized IDL. Parsing the printed text gives back an equal tree
// (apart from `Definition::line`), which the tests check for every vendored file.

/// Writes `[a, b] ` (with a trailing space) or nothing.
pub(crate) struct ExtAttrs<'a>(pub &'a [ExtAttr]);

impl Display for ExtAttrs<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }
        f.write_char('[')?;
        for (i, a) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{a}")?;
        }
        f.write_str("] ")
    }
}

/// Writes `a, b, c`.
pub(crate) struct Args<'a>(pub &'a [Argument]);

impl Display for Args<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for (i, a) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{a}")?;
        }
        Ok(())
    }
}

impl Display for ExtAttr {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        match &self.value {
            ExtAttrValue::None => Ok(()),
            ExtAttrValue::Args(a) => write!(f, "({})", Args(a)),
            ExtAttrValue::Ident(s) | ExtAttrValue::Number(s) => write!(f, "={s}"),
            ExtAttrValue::Wildcard => f.write_str("=*"),
            ExtAttrValue::String(s) => write!(f, "=\"{s}\""),
            ExtAttrValue::List(l) => write!(f, "=({})", l.join(", ")),
            ExtAttrValue::NamedArgs(n, a) => write!(f, "={n}({})", Args(a)),
        }
    }
}

impl Display for Type {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ExtAttrs(&self.ext_attrs))?;
        match &self.kind {
            TypeKind::Named(n) => f.write_str(n)?,
            TypeKind::Sequence(t) => write!(f, "sequence<{t}>")?,
            TypeKind::FrozenArray(t) => write!(f, "FrozenArray<{t}>")?,
            TypeKind::ObservableArray(t) => write!(f, "ObservableArray<{t}>")?,
            TypeKind::Promise(t) => write!(f, "Promise<{t}>")?,
            TypeKind::Record(k, v) => write!(f, "record<{k}, {v}>")?,
            TypeKind::Union(ts) => {
                f.write_char('(')?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" or ")?;
                    }
                    write!(f, "{t}")?;
                }
                f.write_char(')')?;
            }
        }
        if self.nullable {
            f.write_char('?')?;
        }
        Ok(())
    }
}

impl Display for ConstValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ConstValue::Bool(b) => write!(f, "{b}"),
            ConstValue::Integer(s) | ConstValue::Float(s) => f.write_str(s),
        }
    }
}

impl Display for DefaultValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            DefaultValue::Const(c) => write!(f, "{c}"),
            DefaultValue::String(s) => write!(f, "\"{s}\""),
            DefaultValue::EmptySequence => f.write_str("[]"),
            DefaultValue::EmptyDictionary => f.write_str("{}"),
            DefaultValue::Null => f.write_str("null"),
            DefaultValue::Undefined => f.write_str("undefined"),
        }
    }
}

/// Re-applies the leading-underscore escape where an identifier would
/// otherwise read as a keyword in the printed IDL.
fn escape(name: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "async",
        "attribute",
        "callback",
        "const",
        "constructor",
        "deleter",
        "dictionary",
        "enum",
        "getter",
        "includes",
        "inherit",
        "interface",
        "iterable",
        "maplike",
        "mixin",
        "namespace",
        "partial",
        "readonly",
        "required",
        "setlike",
        "setter",
        "static",
        "stringifier",
        "typedef",
        "unrestricted",
        "or",
        "optional",
    ];
    if KEYWORDS.contains(&name) || name.starts_with('_') {
        format!("_{name}")
    } else {
        name.to_owned()
    }
}

impl Display for Argument {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ExtAttrs(&self.ext_attrs))?;
        if self.optional {
            f.write_str("optional ")?;
        }
        write!(f, "{}", self.ty)?;
        if self.variadic {
            f.write_str("...")?;
        }
        write!(f, " {}", escape(&self.name))?;
        if let Some(d) = &self.default {
            write!(f, " = {d}")?;
        }
        Ok(())
    }
}

impl Display for Member {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ExtAttrs(&self.ext_attrs))?;
        match &self.kind {
            MemberKind::Const { ty, name, value } => {
                write!(f, "const {ty} {} = {value};", escape(name))
            }
            MemberKind::Attribute {
                name,
                ty,
                readonly,
                inherit,
                is_static,
                stringifier,
            } => {
                if *stringifier {
                    f.write_str("stringifier ")?;
                }
                if *is_static {
                    f.write_str("static ")?;
                }
                if *inherit {
                    f.write_str("inherit ")?;
                }
                if *readonly {
                    f.write_str("readonly ")?;
                }
                write!(f, "attribute {ty} {};", escape(name))
            }
            MemberKind::Operation {
                name,
                return_type,
                args,
                special,
                is_static,
                stringifier,
            } => {
                if *stringifier {
                    f.write_str("stringifier ")?;
                }
                if *is_static {
                    f.write_str("static ")?;
                }
                match special {
                    Some(Special::Getter) => f.write_str("getter ")?,
                    Some(Special::Setter) => f.write_str("setter ")?,
                    Some(Special::Deleter) => f.write_str("deleter ")?,
                    None => {}
                }
                match name {
                    Some(n) => write!(f, "{return_type} {}", escape(n))?,
                    None => write!(f, "{return_type} ")?,
                }
                write!(f, "({});", Args(args))
            }
            MemberKind::Constructor { args } => write!(f, "constructor({});", Args(args)),
            MemberKind::Stringifier => f.write_str("stringifier;"),
            MemberKind::Iterable {
                is_async,
                key,
                value,
                args,
            } => {
                if *is_async {
                    f.write_str("async ")?;
                }
                f.write_str("iterable<")?;
                if let Some(k) = key {
                    write!(f, "{k}, ")?;
                }
                write!(f, "{value}>")?;
                if let Some(a) = args {
                    write!(f, "({})", Args(a))?;
                }
                f.write_char(';')
            }
            MemberKind::Maplike {
                readonly,
                key,
                value,
            } => {
                if *readonly {
                    f.write_str("readonly ")?;
                }
                write!(f, "maplike<{key}, {value}>;")
            }
            MemberKind::Setlike { readonly, value } => {
                if *readonly {
                    f.write_str("readonly ")?;
                }
                write!(f, "setlike<{value}>;")
            }
        }
    }
}

impl Display for DictionaryMember {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ExtAttrs(&self.ext_attrs))?;
        if self.required {
            f.write_str("required ")?;
        }
        write!(f, "{} {}", self.ty, escape(&self.name))?;
        if let Some(d) = &self.default {
            write!(f, " = {d}")?;
        }
        f.write_char(';')
    }
}

fn members<T: Display>(f: &mut Formatter<'_>, ms: &[T]) -> fmt::Result {
    f.write_str(" {\n")?;
    for m in ms {
        writeln!(f, "  {m}")?;
    }
    f.write_str("};")
}

impl Display for Definition {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ExtAttrs(&self.ext_attrs))?;
        match &self.kind {
            DefinitionKind::Interface(i) => {
                if i.partial {
                    f.write_str("partial ")?;
                }
                match i.kind {
                    InterfaceKind::Interface => f.write_str("interface ")?,
                    InterfaceKind::Mixin => f.write_str("interface mixin ")?,
                    InterfaceKind::Callback => f.write_str("callback interface ")?,
                }
                f.write_str(&escape(&i.name))?;
                if let Some(p) = &i.inherits {
                    write!(f, " : {}", escape(p))?;
                }
                members(f, &i.members)
            }
            DefinitionKind::Namespace(n) => {
                if n.partial {
                    f.write_str("partial ")?;
                }
                write!(f, "namespace {}", escape(&n.name))?;
                members(f, &n.members)
            }
            DefinitionKind::Dictionary(d) => {
                if d.partial {
                    f.write_str("partial ")?;
                }
                write!(f, "dictionary {}", escape(&d.name))?;
                if let Some(p) = &d.inherits {
                    write!(f, " : {}", escape(p))?;
                }
                members(f, &d.members)
            }
            DefinitionKind::Enum { name, values } => {
                write!(f, "enum {} {{ ", escape(name))?;
                for (i, v) in values.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "\"{v}\"")?;
                }
                f.write_str(" };")
            }
            DefinitionKind::Typedef { ty, name } => {
                write!(f, "typedef {ty} {};", escape(name))
            }
            DefinitionKind::Callback {
                name,
                return_type,
                args,
            } => write!(
                f,
                "callback {} = {return_type} ({});",
                escape(name),
                Args(args)
            ),
            DefinitionKind::Includes { target, mixin } => {
                write!(f, "{} includes {};", escape(target), escape(mixin))
            }
        }
    }
}
