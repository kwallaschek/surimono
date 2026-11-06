//! The values that cross `call(op, generation, handle, ...args)`.
//!
//! The generated glue converts every argument by the WebIDL rules in
//! JavaScript, so [`Args`] only reads primitives back. A value of the wrong
//! shape can't come from the glue; it is a `TypeError`, not a panic.

use surimono_core::rquickjs::{self, Ctx, IntoJs, Value};
use surimono_webidl::runtime::Value as IdlValue;
use surimono_webidl::runtime::{BindingError, BindingResult, ObjectId};

/// The arguments of one call.
pub(crate) struct Args<'js> {
    values: Vec<Value<'js>>,
}

fn bad(i: usize) -> BindingError {
    BindingError::TypeError(format!("argument {i} has the wrong type"))
}

// The accessors follow the types in the IDL; not all have callers yet.
#[allow(dead_code)]
impl<'js> Args<'js> {
    pub(crate) fn new(values: Vec<Value<'js>>) -> Self {
        Self { values }
    }

    fn get(&self, i: usize) -> Option<&Value<'js>> {
        self.values.get(i)
    }

    fn is_none(&self, i: usize) -> bool {
        self.get(i).is_none_or(|v| v.is_undefined() || v.is_null())
    }

    fn to_str(v: &Value<'js>, i: usize) -> BindingResult<String> {
        v.as_string()
            .and_then(|s| s.to_string().ok())
            .ok_or_else(|| bad(i))
    }

    pub(crate) fn string(&self, i: usize) -> BindingResult<String> {
        self.get(i)
            .ok_or_else(|| bad(i))
            .and_then(|v| Self::to_str(v, i))
    }

    pub(crate) fn opt_string(&self, i: usize) -> BindingResult<Option<String>> {
        if self.is_none(i) {
            return Ok(None);
        }
        self.string(i).map(Some)
    }

    pub(crate) fn boolean(&self, i: usize) -> BindingResult<bool> {
        self.get(i).and_then(Value::as_bool).ok_or_else(|| bad(i))
    }

    pub(crate) fn opt_boolean(&self, i: usize) -> BindingResult<Option<bool>> {
        if self.is_none(i) {
            return Ok(None);
        }
        self.boolean(i).map(Some)
    }

    fn number(&self, i: usize) -> BindingResult<f64> {
        self.get(i).and_then(Value::as_number).ok_or_else(|| bad(i))
    }

    pub(crate) fn uint(&self, i: usize) -> BindingResult<u32> {
        self.number(i).map(|n| n as u32)
    }

    pub(crate) fn int(&self, i: usize) -> BindingResult<i32> {
        self.number(i).map(|n| n as i32)
    }

    pub(crate) fn ushort(&self, i: usize) -> BindingResult<u16> {
        self.number(i).map(|n| n as u16)
    }

    pub(crate) fn short(&self, i: usize) -> BindingResult<i16> {
        self.number(i).map(|n| n as i16)
    }

    pub(crate) fn id(&self, i: usize) -> BindingResult<ObjectId> {
        self.number(i).map(|n| ObjectId(n as u64))
    }

    /// A nullable interface: 0 is null, and so is an optional one's default.
    pub(crate) fn opt_id(&self, i: usize) -> BindingResult<Option<ObjectId>> {
        if self.is_none(i) {
            return Ok(None);
        }
        self.id(i).map(|id| (id.0 != 0).then_some(id))
    }

    fn idl_value(v: &Value<'js>, i: usize) -> BindingResult<IdlValue> {
        if v.is_undefined() {
            Ok(IdlValue::Undefined)
        } else if v.is_null() {
            Ok(IdlValue::Null)
        } else if let Some(b) = v.as_bool() {
            Ok(IdlValue::Bool(b))
        } else if let Some(n) = v.as_number() {
            // Only interfaces cross as numbers among `Value` arguments.
            Ok(IdlValue::Object(ObjectId(n as u64)))
        } else {
            Self::to_str(v, i).map(IdlValue::String)
        }
    }

    pub(crate) fn value(&self, i: usize) -> BindingResult<IdlValue> {
        match self.get(i) {
            None => Ok(IdlValue::Undefined),
            Some(v) => Self::idl_value(v, i),
        }
    }

    /// A variadic of strings from `i` on.
    pub(crate) fn strings(&self, i: usize) -> BindingResult<Vec<String>> {
        (i..self.values.len()).map(|k| self.string(k)).collect()
    }

    /// A variadic of `(Node or DOMString)` from `i` on.
    pub(crate) fn values(&self, i: usize) -> BindingResult<Vec<IdlValue>> {
        (i..self.values.len()).map(|k| self.value(k)).collect()
    }
}

/// What a member returns, ready to become a JavaScript primitive. Interfaces
/// cross as handle numbers, 0 for null; the glue wraps them.
pub(crate) enum Ret {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
}

impl Ret {
    pub(crate) fn into_js<'js>(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        match self {
            Ret::Undefined => Ok(Value::new_undefined(ctx.clone())),
            Ret::Null => Ok(Value::new_null(ctx.clone())),
            Ret::Bool(b) => Ok(Value::new_bool(ctx.clone(), b)),
            Ret::Number(n) => Ok(Value::new_number(ctx.clone(), n)),
            Ret::Str(s) => s.into_js(ctx),
        }
    }
}

impl From<()> for Ret {
    fn from((): ()) -> Self {
        Ret::Undefined
    }
}

impl From<bool> for Ret {
    fn from(b: bool) -> Self {
        Ret::Bool(b)
    }
}

impl From<i16> for Ret {
    fn from(n: i16) -> Self {
        Ret::Number(f64::from(n))
    }
}

impl From<u16> for Ret {
    fn from(n: u16) -> Self {
        Ret::Number(f64::from(n))
    }
}

impl From<u32> for Ret {
    fn from(n: u32) -> Self {
        Ret::Number(f64::from(n))
    }
}

impl From<String> for Ret {
    fn from(s: String) -> Self {
        Ret::Str(s)
    }
}

impl From<Option<String>> for Ret {
    fn from(s: Option<String>) -> Self {
        s.map_or(Ret::Null, Ret::Str)
    }
}

impl From<ObjectId> for Ret {
    fn from(id: ObjectId) -> Self {
        Ret::Number(id.0 as f64)
    }
}

impl From<Option<ObjectId>> for Ret {
    fn from(id: Option<ObjectId>) -> Self {
        Ret::Number(id.map_or(0.0, |i| i.0 as f64))
    }
}

impl From<Vec<String>> for Ret {
    /// `sequence<DOMString>`: names, newline-joined for `nameArray`.
    fn from(names: Vec<String>) -> Self {
        Ret::Str(names.join("\n"))
    }
}

impl From<IdlValue> for Ret {
    fn from(v: IdlValue) -> Self {
        match v {
            IdlValue::Undefined => Ret::Undefined,
            IdlValue::Null => Ret::Null,
            IdlValue::Bool(b) => Ret::Bool(b),
            IdlValue::Number(n) => Ret::Number(n),
            IdlValue::String(s) => Ret::Str(s),
            IdlValue::Object(id) => Ret::Number(id.0 as f64),
        }
    }
}
