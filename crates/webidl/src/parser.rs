//! A recursive-descent parser for the WebIDL grammar
//! (<https://webidl.spec.whatwg.org/#idl-grammar>), as used by the specs that
//! w3c/webref extracts. It never panics: every malformed input is a
//! [`ParseError`] with a line and column.

use crate::ParseError;
use crate::ast::*;
use crate::lexer::{Token, TokenKind, tokenize};

/// Deepest nesting of types and extended attributes accepted. Spec IDL nests
/// a few levels; the cap keeps hostile input from exhausting the stack.
/// Measured in a debug build, the worst pattern at this cap needs about
/// 144 KiB of stack (release: about 40 KiB), so parsing fits a 256 KiB
/// thread; `tests/stack.rs` checks that on such a thread. It was 64, which
/// needed about 384 KiB.
const MAX_DEPTH: u32 = 32;

/// Parses one IDL source into its definitions, in source order.
pub fn parse(src: &str) -> Result<Vec<Definition>, ParseError> {
    let tokens = tokenize(src)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        depth: 0,
    };
    let mut defs = Vec::new();
    while !p.at_end() {
        defs.push(p.definition()?);
    }
    Ok(defs)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: u32,
}

/// Strips WebIDL's leading-underscore escape.
fn unescape(s: &str) -> String {
    s.strip_prefix('_').unwrap_or(s).to_owned()
}

impl Parser {
    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos).map(|t| &t.kind)
    }

    fn peek_at(&self, n: usize) -> Option<&TokenKind> {
        self.tokens.get(self.pos + n).map(|t| &t.kind)
    }

    fn peek_ident(&self) -> Option<&str> {
        match self.peek() {
            Some(TokenKind::Ident(s)) => Some(s),
            _ => None,
        }
    }

    fn peek_ident_at(&self, n: usize) -> Option<&str> {
        match self.peek_at(n) {
            Some(TokenKind::Ident(s)) => Some(s),
            _ => None,
        }
    }

    fn is_char(&self, c: char) -> bool {
        self.peek() == Some(&TokenKind::Other(c))
    }

    fn is_keyword(&self, k: &str) -> bool {
        self.peek_ident() == Some(k)
    }

    fn line(&self) -> u32 {
        self.tokens
            .get(self.pos)
            .or(self.tokens.last())
            .map_or(1, |t| t.line)
    }

    fn error(&self, expected: &str) -> ParseError {
        let found = match self.peek() {
            None => "end of input".to_owned(),
            Some(TokenKind::Ident(s)) => format!("`{s}`"),
            Some(TokenKind::Integer(s) | TokenKind::Decimal(s)) => format!("number `{s}`"),
            Some(TokenKind::Str(s)) => format!("string \"{s}\""),
            Some(TokenKind::Ellipsis) => "`...`".to_owned(),
            Some(TokenKind::Other(c)) => format!("`{c}`"),
        };
        let (line, column) = self
            .tokens
            .get(self.pos)
            .or(self.tokens.last())
            .map_or((1, 1), |t| (t.line, t.column));
        ParseError::new(line, column, format!("expected {expected}, found {found}"))
    }

    fn eat_char(&mut self, c: char) -> bool {
        if self.is_char(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_char(&mut self, c: char) -> Result<(), ParseError> {
        if self.eat_char(c) {
            Ok(())
        } else {
            Err(self.error(&format!("`{c}`")))
        }
    }

    fn eat_keyword(&mut self, k: &str) -> bool {
        if self.is_keyword(k) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_keyword(&mut self, k: &str) -> Result<(), ParseError> {
        if self.eat_keyword(k) {
            Ok(())
        } else {
            Err(self.error(&format!("`{k}`")))
        }
    }

    /// An identifier in a name position (keywords allowed), unescaped.
    fn name(&mut self) -> Result<String, ParseError> {
        match self.peek() {
            Some(TokenKind::Ident(s)) => {
                let s = unescape(s);
                self.pos += 1;
                Ok(s)
            }
            _ => Err(self.error("an identifier")),
        }
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("less deeply nested IDL"));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // -- Definitions -------------------------------------------------------

    fn definition(&mut self) -> Result<Definition, ParseError> {
        let ext_attrs = self.ext_attrs()?;
        let line = self.line();
        let kind = if self.eat_keyword("callback") {
            if self.eat_keyword("interface") {
                DefinitionKind::Interface(self.interface_rest(InterfaceKind::Callback, false)?)
            } else {
                let name = self.name()?;
                self.expect_char('=')?;
                let return_type = self.ty()?;
                self.expect_char('(')?;
                let args = self.args()?;
                self.expect_char(')')?;
                self.expect_char(';')?;
                DefinitionKind::Callback {
                    name,
                    return_type,
                    args,
                }
            }
        } else if self.eat_keyword("interface") {
            if self.eat_keyword("mixin") {
                DefinitionKind::Interface(self.interface_rest(InterfaceKind::Mixin, false)?)
            } else {
                DefinitionKind::Interface(self.interface_rest(InterfaceKind::Interface, false)?)
            }
        } else if self.eat_keyword("partial") {
            if self.eat_keyword("interface") {
                let kind = if self.eat_keyword("mixin") {
                    InterfaceKind::Mixin
                } else {
                    InterfaceKind::Interface
                };
                DefinitionKind::Interface(self.interface_rest(kind, true)?)
            } else if self.eat_keyword("dictionary") {
                DefinitionKind::Dictionary(self.dictionary_rest(true)?)
            } else if self.eat_keyword("namespace") {
                DefinitionKind::Namespace(self.namespace_rest(true)?)
            } else {
                return Err(self.error("`interface`, `dictionary` or `namespace`"));
            }
        } else if self.eat_keyword("namespace") {
            DefinitionKind::Namespace(self.namespace_rest(false)?)
        } else if self.eat_keyword("dictionary") {
            DefinitionKind::Dictionary(self.dictionary_rest(false)?)
        } else if self.eat_keyword("enum") {
            let name = self.name()?;
            self.expect_char('{')?;
            let mut values = Vec::new();
            loop {
                if self.eat_char('}') {
                    break;
                }
                match self.peek() {
                    Some(TokenKind::Str(s)) => {
                        values.push(s.clone());
                        self.pos += 1;
                    }
                    _ => return Err(self.error("a string or `}`")),
                }
                if !self.eat_char(',') {
                    self.expect_char('}')?;
                    break;
                }
            }
            if values.is_empty() {
                return Err(self.error("at least one enum value"));
            }
            self.expect_char(';')?;
            DefinitionKind::Enum { name, values }
        } else if self.eat_keyword("typedef") {
            let ty = self.ty()?;
            let name = self.name()?;
            self.expect_char(';')?;
            DefinitionKind::Typedef { ty, name }
        } else if self.peek_ident().is_some() && self.peek_ident_at(1) == Some("includes") {
            let target = self.name()?;
            self.expect_keyword("includes")?;
            let mixin = self.name()?;
            self.expect_char(';')?;
            DefinitionKind::Includes { target, mixin }
        } else {
            return Err(self.error("a definition"));
        };
        Ok(Definition {
            ext_attrs,
            kind,
            line,
        })
    }

    fn inheritance(&mut self) -> Result<Option<String>, ParseError> {
        if self.eat_char(':') {
            Ok(Some(self.name()?))
        } else {
            Ok(None)
        }
    }

    fn interface_rest(
        &mut self,
        kind: InterfaceKind,
        partial: bool,
    ) -> Result<Interface, ParseError> {
        let name = self.name()?;
        let inherits = if kind == InterfaceKind::Interface && !partial {
            self.inheritance()?
        } else {
            None
        };
        let members = self.member_block()?;
        Ok(Interface {
            kind,
            partial,
            name,
            inherits,
            members,
        })
    }

    fn namespace_rest(&mut self, partial: bool) -> Result<Namespace, ParseError> {
        let name = self.name()?;
        let members = self.member_block()?;
        Ok(Namespace {
            partial,
            name,
            members,
        })
    }

    fn member_block(&mut self) -> Result<Vec<Member>, ParseError> {
        self.expect_char('{')?;
        let mut members = Vec::new();
        while !self.eat_char('}') {
            members.push(self.member()?);
        }
        self.expect_char(';')?;
        Ok(members)
    }

    fn dictionary_rest(&mut self, partial: bool) -> Result<Dictionary, ParseError> {
        let name = self.name()?;
        let inherits = if partial { None } else { self.inheritance()? };
        self.expect_char('{')?;
        let mut members = Vec::new();
        while !self.eat_char('}') {
            let ext_attrs = self.ext_attrs()?;
            let required = self.eat_keyword("required");
            let ty = self.ty()?;
            let name = self.name()?;
            let default = if self.eat_char('=') {
                Some(self.default_value()?)
            } else {
                None
            };
            self.expect_char(';')?;
            members.push(DictionaryMember {
                ext_attrs,
                required,
                ty,
                name,
                default,
            });
        }
        self.expect_char(';')?;
        Ok(Dictionary {
            partial,
            name,
            inherits,
            members,
        })
    }

    // -- Members -----------------------------------------------------------

    fn member(&mut self) -> Result<Member, ParseError> {
        let ext_attrs = self.ext_attrs()?;
        let kind = self.member_kind()?;
        Ok(Member { ext_attrs, kind })
    }

    fn member_kind(&mut self) -> Result<MemberKind, ParseError> {
        if self.eat_keyword("const") {
            let ty = self.ty()?;
            let name = self.name()?;
            self.expect_char('=')?;
            let value = self.const_value()?;
            self.expect_char(';')?;
            return Ok(MemberKind::Const { ty, name, value });
        }
        if self.is_keyword("constructor") && self.peek_at(1) == Some(&TokenKind::Other('(')) {
            self.pos += 1;
            self.expect_char('(')?;
            let args = self.args()?;
            self.expect_char(')')?;
            self.expect_char(';')?;
            return Ok(MemberKind::Constructor { args });
        }
        if self.eat_keyword("stringifier") {
            if self.eat_char(';') {
                return Ok(MemberKind::Stringifier);
            }
            if self.is_keyword("attribute") || self.is_keyword("readonly") {
                return self.attribute(false, true);
            }
            return self.operation(None, false, true);
        }
        if self.eat_keyword("static") {
            if self.is_keyword("attribute") || self.is_keyword("readonly") {
                return self.attribute(true, false);
            }
            return self.operation(None, true, false);
        }
        if self.is_keyword("inherit")
            || self.is_keyword("attribute")
            || (self.is_keyword("readonly") && self.peek_ident_at(1) == Some("attribute"))
        {
            return self.attribute(false, false);
        }
        let readonly = self.is_keyword("readonly")
            && matches!(self.peek_ident_at(1), Some("maplike" | "setlike"));
        if readonly {
            self.pos += 1;
        }
        if self.eat_keyword("maplike") {
            self.expect_char('<')?;
            let key = self.ty()?;
            self.expect_char(',')?;
            let value = self.ty()?;
            self.expect_char('>')?;
            self.expect_char(';')?;
            return Ok(MemberKind::Maplike {
                readonly,
                key,
                value,
            });
        }
        if self.eat_keyword("setlike") {
            self.expect_char('<')?;
            let value = self.ty()?;
            self.expect_char('>')?;
            self.expect_char(';')?;
            return Ok(MemberKind::Setlike { readonly, value });
        }
        if self.is_keyword("iterable")
            || self.is_keyword("async_iterable")
            || (self.is_keyword("async") && self.peek_ident_at(1) == Some("iterable"))
        {
            let is_async = !self.is_keyword("iterable");
            if self.eat_keyword("async") {
                self.expect_keyword("iterable")?;
            } else {
                self.pos += 1;
            }
            self.expect_char('<')?;
            let first = self.ty()?;
            let (key, value) = if self.eat_char(',') {
                (Some(first), self.ty()?)
            } else {
                (None, first)
            };
            self.expect_char('>')?;
            let args = if is_async && self.eat_char('(') {
                let a = self.args()?;
                self.expect_char(')')?;
                Some(a)
            } else {
                None
            };
            self.expect_char(';')?;
            return Ok(MemberKind::Iterable {
                is_async,
                key,
                value,
                args,
            });
        }
        let special = if self.eat_keyword("getter") {
            Some(Special::Getter)
        } else if self.eat_keyword("setter") {
            Some(Special::Setter)
        } else if self.eat_keyword("deleter") {
            Some(Special::Deleter)
        } else {
            None
        };
        self.operation(special, false, false)
    }

    fn attribute(&mut self, is_static: bool, stringifier: bool) -> Result<MemberKind, ParseError> {
        let inherit = self.eat_keyword("inherit");
        let readonly = self.eat_keyword("readonly");
        self.expect_keyword("attribute")?;
        let ty = self.ty()?;
        let name = self.name()?;
        self.expect_char(';')?;
        Ok(MemberKind::Attribute {
            name,
            ty,
            readonly,
            inherit,
            is_static,
            stringifier,
        })
    }

    fn operation(
        &mut self,
        special: Option<Special>,
        is_static: bool,
        stringifier: bool,
    ) -> Result<MemberKind, ParseError> {
        let return_type = self.ty()?;
        let name = if self.is_char('(') {
            if special.is_none() {
                return Err(self.error("an operation name"));
            }
            None
        } else {
            Some(self.name()?)
        };
        self.expect_char('(')?;
        let args = self.args()?;
        self.expect_char(')')?;
        self.expect_char(';')?;
        Ok(MemberKind::Operation {
            name,
            return_type,
            args,
            special,
            is_static,
            stringifier,
        })
    }

    /// A possibly empty, comma-separated argument list (without parentheses).
    fn args(&mut self) -> Result<Vec<Argument>, ParseError> {
        let mut args = Vec::new();
        if self.is_char(')') {
            return Ok(args);
        }
        loop {
            let ext_attrs = self.ext_attrs()?;
            let optional = self.eat_keyword("optional");
            let ty = self.ty()?;
            let variadic = !optional && self.peek() == Some(&TokenKind::Ellipsis);
            if variadic {
                self.pos += 1;
            }
            let name = self.name()?;
            let default = if optional && self.eat_char('=') {
                Some(self.default_value()?)
            } else {
                None
            };
            args.push(Argument {
                ext_attrs,
                optional,
                ty,
                variadic,
                name,
                default,
            });
            if !self.eat_char(',') {
                return Ok(args);
            }
        }
    }

    // -- Values ------------------------------------------------------------

    fn const_value(&mut self) -> Result<ConstValue, ParseError> {
        let v = match self.peek() {
            Some(TokenKind::Ident(s)) => match s.as_str() {
                "true" => ConstValue::Bool(true),
                "false" => ConstValue::Bool(false),
                "Infinity" | "-Infinity" | "NaN" => ConstValue::Float(s.clone()),
                _ => return Err(self.error("a constant value")),
            },
            Some(TokenKind::Integer(s)) => ConstValue::Integer(s.clone()),
            Some(TokenKind::Decimal(s)) => ConstValue::Float(s.clone()),
            _ => return Err(self.error("a constant value")),
        };
        self.pos += 1;
        Ok(v)
    }

    fn default_value(&mut self) -> Result<DefaultValue, ParseError> {
        match self.peek() {
            Some(TokenKind::Str(s)) => {
                let v = DefaultValue::String(s.clone());
                self.pos += 1;
                Ok(v)
            }
            Some(TokenKind::Other('[')) => {
                self.pos += 1;
                self.expect_char(']')?;
                Ok(DefaultValue::EmptySequence)
            }
            Some(TokenKind::Other('{')) => {
                self.pos += 1;
                self.expect_char('}')?;
                Ok(DefaultValue::EmptyDictionary)
            }
            Some(TokenKind::Ident(s)) if s == "null" => {
                self.pos += 1;
                Ok(DefaultValue::Null)
            }
            Some(TokenKind::Ident(s)) if s == "undefined" => {
                self.pos += 1;
                Ok(DefaultValue::Undefined)
            }
            _ => Ok(DefaultValue::Const(self.const_value()?)),
        }
    }

    // -- Extended attributes -----------------------------------------------

    fn ext_attrs(&mut self) -> Result<Vec<ExtAttr>, ParseError> {
        let mut out = Vec::new();
        if !self.eat_char('[') {
            return Ok(out);
        }
        self.enter()?;
        loop {
            out.push(self.ext_attr()?);
            if !self.eat_char(',') {
                break;
            }
        }
        self.expect_char(']')?;
        self.leave();
        Ok(out)
    }

    fn ext_attr(&mut self) -> Result<ExtAttr, ParseError> {
        let name = self.name()?;
        let value = if self.eat_char('(') {
            let a = self.args()?;
            self.expect_char(')')?;
            ExtAttrValue::Args(a)
        } else if self.eat_char('=') {
            match self.peek().cloned() {
                Some(TokenKind::Other('*')) => {
                    self.pos += 1;
                    ExtAttrValue::Wildcard
                }
                Some(TokenKind::Str(s)) => {
                    self.pos += 1;
                    ExtAttrValue::String(s)
                }
                Some(TokenKind::Integer(s) | TokenKind::Decimal(s)) => {
                    self.pos += 1;
                    ExtAttrValue::Number(s)
                }
                Some(TokenKind::Other('(')) => {
                    self.pos += 1;
                    let mut list = Vec::new();
                    loop {
                        match self.peek().cloned() {
                            Some(TokenKind::Ident(s)) => list.push(unescape(&s)),
                            Some(TokenKind::Str(s)) => list.push(format!("\"{s}\"")),
                            Some(TokenKind::Integer(s) | TokenKind::Decimal(s)) => list.push(s),
                            _ => return Err(self.error("an identifier, string or number")),
                        }
                        self.pos += 1;
                        if !self.eat_char(',') {
                            break;
                        }
                    }
                    self.expect_char(')')?;
                    ExtAttrValue::List(list)
                }
                Some(TokenKind::Ident(_)) => {
                    let id = self.name()?;
                    if self.eat_char('(') {
                        let a = self.args()?;
                        self.expect_char(')')?;
                        ExtAttrValue::NamedArgs(id, a)
                    } else {
                        ExtAttrValue::Ident(id)
                    }
                }
                _ => return Err(self.error("an extended attribute value")),
            }
        } else {
            ExtAttrValue::None
        };
        Ok(ExtAttr { name, value })
    }

    // -- Types -------------------------------------------------------------

    fn ty(&mut self) -> Result<Type, ParseError> {
        self.enter()?;
        let ext_attrs = self.ext_attrs()?;
        let kind = if self.eat_char('(') {
            let mut members = vec![self.ty()?];
            while self.eat_keyword("or") {
                members.push(self.ty()?);
            }
            self.expect_char(')')?;
            if members.len() < 2 {
                return Err(self.error("`or`"));
            }
            TypeKind::Union(members)
        } else {
            self.single_type()?
        };
        let nullable = self.eat_char('?');
        self.leave();
        Ok(Type {
            ext_attrs,
            kind,
            nullable,
        })
    }

    fn generic1(&mut self) -> Result<Box<Type>, ParseError> {
        self.expect_char('<')?;
        let t = self.ty()?;
        self.expect_char('>')?;
        Ok(Box::new(t))
    }

    fn single_type(&mut self) -> Result<TypeKind, ParseError> {
        let Some(word) = self.peek_ident().map(str::to_owned) else {
            return Err(self.error("a type"));
        };
        self.pos += 1;
        let kind = match word.as_str() {
            "sequence" => TypeKind::Sequence(self.generic1()?),
            "FrozenArray" => TypeKind::FrozenArray(self.generic1()?),
            "ObservableArray" => TypeKind::ObservableArray(self.generic1()?),
            "Promise" => TypeKind::Promise(self.generic1()?),
            "record" => {
                self.expect_char('<')?;
                let k = self.ty()?;
                self.expect_char(',')?;
                let v = self.ty()?;
                self.expect_char('>')?;
                TypeKind::Record(Box::new(k), Box::new(v))
            }
            "unsigned" => {
                if self.eat_keyword("short") {
                    TypeKind::Named("unsigned short".into())
                } else if self.eat_keyword("long") {
                    if self.eat_keyword("long") {
                        TypeKind::Named("unsigned long long".into())
                    } else {
                        TypeKind::Named("unsigned long".into())
                    }
                } else {
                    return Err(self.error("`short` or `long`"));
                }
            }
            "long" => {
                if self.eat_keyword("long") {
                    TypeKind::Named("long long".into())
                } else {
                    TypeKind::Named("long".into())
                }
            }
            "unrestricted" => {
                if self.eat_keyword("float") {
                    TypeKind::Named("unrestricted float".into())
                } else if self.eat_keyword("double") {
                    TypeKind::Named("unrestricted double".into())
                } else {
                    return Err(self.error("`float` or `double`"));
                }
            }
            _ => TypeKind::Named(unescape(&word)),
        };
        Ok(kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(src: &str) -> Definition {
        let mut d = parse(src).unwrap();
        assert_eq!(d.len(), 1, "{src}");
        d.remove(0)
    }

    fn round_trip(src: &str) {
        let a = parse(src).unwrap();
        let printed: String = a.iter().map(|d| format!("{d}\n")).collect();
        let b = parse(&printed).unwrap_or_else(|e| panic!("{e}\n{printed}"));
        let strip = |v: Vec<Definition>| {
            v.into_iter()
                .map(|mut d| {
                    d.line = 0;
                    d
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(strip(a), strip(b), "{printed}");
    }

    #[test]
    fn interface_with_every_member_kind() {
        let src = r#"
            [Exposed=(Window,Worker), LegacyWindowAlias=webkitURL]
            interface Foo : Bar {
              constructor(optional DOMString x = "a", long... rest);
              const unsigned short A = 0x10;
              const double B = -Infinity;
              [SameObject] readonly attribute DOMTokenList classList;
              [CEReactions] attribute [LegacyNullToEmptyString] DOMString text;
              inherit attribute EventHandler onx;
              static readonly attribute long count;
              stringifier attribute USVString href;
              stringifier;
              getter Element? (unsigned long index);
              setter undefined (DOMString name, any value);
              deleter undefined (DOMString name);
              getter DOMString? item(unsigned long index);
              static Foo create(optional FooInit init = {});
              Promise<sequence<(Node or DOMString)?>> run(record<DOMString, long long> r, optional sequence<long> s = []);
              iterable<DOMString>;
              async iterable<long, DOMString>(optional long a);
              readonly maplike<DOMString, long>;
              setlike<long>;
              undefined _interface(boolean required);
            };
        "#;
        let d = one(src);
        let DefinitionKind::Interface(i) = &d.kind else {
            panic!()
        };
        assert_eq!(i.name, "Foo");
        assert_eq!(i.inherits.as_deref(), Some("Bar"));
        assert_eq!(i.members.len(), 20);
        assert_eq!(
            d.ext_attrs[0].value,
            ExtAttrValue::List(vec!["Window".into(), "Worker".into()])
        );
        let MemberKind::Operation { name, .. } = &i.members[19].kind else {
            panic!()
        };
        assert_eq!(name.as_deref(), Some("interface"));
        round_trip(src);
    }

    #[test]
    fn other_definitions() {
        let src = r#"
            interface mixin M { readonly attribute long x; };
            partial interface mixin M { undefined f(); };
            callback interface CI { undefined handleEvent(Event e); };
            callback CB = any (any... args);
            dictionary D : E { required long a; DOMString b = "x"; sequence<long> c = []; boolean? d = null; };
            partial dictionary D { long e; };
            enum En { "a", "b", };
            typedef (unsigned long long or [EnforceRange] long) T;
            namespace console { undefined log(any... data); };
            partial namespace console { readonly attribute long x; };
            Foo includes M;
            [LegacyFactoryFunction=Image(optional unsigned long width), Exposed=*, X="s", Y=1]
            partial interface Foo { };
        "#;
        let defs = parse(src).unwrap();
        assert_eq!(defs.len(), 12);
        assert!(matches!(defs[10].kind, DefinitionKind::Includes { .. }));
        round_trip(src);
    }

    #[test]
    fn errors_have_positions() {
        let e = parse("interface A {\n  attribute long;\n};").unwrap_err();
        assert_eq!((e.line, e.column), (2, 17));
        assert!(e.message.contains("expected an identifier"), "{e}");
        assert!(parse("interface A { };x").is_err());
        assert!(parse("interface A {").is_err());
        assert!(parse("enum E { };").is_err());
        assert!(parse("typedef (long) T;").is_err());
        assert!(parse("interface A { long (); };").is_err());
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let src = format!(
            "typedef {}long{} T;",
            "sequence<".repeat(10_000),
            ">".repeat(10_000)
        );
        assert!(parse(&src).is_err());
        let src = format!("{}interface A {{}};", "[A(".repeat(10_000));
        assert!(parse(&src).is_err());
    }

    #[test]
    fn nesting_up_to_the_cap_parses() {
        // `typedef` + n `sequence<` enters `ty` n + 1 times.
        let nested =
            |n: usize| format!("typedef {}long{} T;", "sequence<".repeat(n), ">".repeat(n));
        let cap = MAX_DEPTH as usize;
        assert!(parse(&nested(cap - 1)).is_ok());
        let e = parse(&nested(cap)).unwrap_err();
        assert!(e.message.contains("less deeply nested"), "{e}");
    }
}
