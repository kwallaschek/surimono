//! Types the generated stubs ([`crate::stubs`]) are written against.
//!
//! JS never holds a Rust pointer: every platform object crosses the boundary
//! as an [`ObjectId`], and the host (phase 2: `surimono-dom`) looks it up in
//! its own tables. See `crates/webidl/DESIGN.md`.

use std::fmt;

/// A handle to a platform object (a DOM node, a `DOMTokenList`, an event …)
/// owned by the host. Only the host gives these meaning; a stale or forged id
/// is an error, never undefined behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(pub u64);

/// A JS value as the stubs see it, for IDL types not yet given a precise
/// Rust type (`any`, `object`, unions, dictionaries, callbacks, promises,
/// buffer sources and names outside the vendored IDL).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `undefined`
    Undefined,
    /// `null`
    Null,
    /// A boolean.
    Bool(bool),
    /// A number.
    Number(f64),
    /// A string.
    String(String),
    /// A platform object.
    Object(ObjectId),
}

/// An error a binding returns; the JS layer turns it into an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingError {
    /// Throw a `TypeError`.
    TypeError(String),
    /// Throw a `DOMException` with this name (for example `"NotFoundError"`).
    DomException {
        /// The DOMException name.
        name: &'static str,
        /// The message.
        message: String,
    },
    /// The member exists in IDL but has no implementation yet. The JS layer
    /// throws a `TypeError` naming it.
    NotImplemented(&'static str),
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingError::TypeError(m) => write!(f, "TypeError: {m}"),
            BindingError::DomException { name, message } => write!(f, "{name}: {message}"),
            BindingError::NotImplemented(what) => write!(f, "{what} is not implemented"),
        }
    }
}

impl std::error::Error for BindingError {}

/// The result of every generated binding method.
pub type BindingResult<T> = Result<T, BindingError>;

/// What kind of member a [`MemberInfo`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberInfoKind {
    /// A constant (exposed on the interface object and prototype).
    Const,
    /// An attribute; `readonly` attributes have no setter method.
    Attribute {
        /// `readonly`
        readonly: bool,
        /// `static`
        is_static: bool,
    },
    /// A regular or static operation (all its overloads).
    Operation {
        /// `static`
        is_static: bool,
        /// Number of overloads, each its own Rust method.
        overloads: u8,
    },
    /// A constructor (all its overloads).
    Constructor {
        /// Number of overloads, each its own Rust method.
        overloads: u8,
    },
    /// An anonymous getter, setter or deleter, or a bare `stringifier;`.
    Special,
    /// `iterable`, `maplike` or `setlike`; not generated yet.
    Collection,
}

/// One member of a generated interface, for registering it with the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberInfo {
    /// The name JS sees (empty for anonymous special members).
    pub idl_name: &'static str,
    /// The Rust method name (the getter, for attributes; the first overload,
    /// for operations). Empty for members with no method yet.
    pub rust_name: &'static str,
    /// What kind of member.
    pub kind: MemberInfoKind,
}

/// One generated interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceInfo {
    /// Interface name.
    pub name: &'static str,
    /// Inherited interface, if any.
    pub parent: Option<&'static str>,
    /// Vendored files that contributed members.
    pub files: &'static [&'static str],
    /// Members in merge order.
    pub members: &'static [MemberInfo],
}
