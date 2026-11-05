//! The `style` attribute as `CSSStyleDeclaration` sees it (CSSOM §6.6),
//! kept as text in the attribute, which is what gets serialized and printed.
//!
//! cssparser tokenizes (CSS Syntax): comments, strings, escapes and blocks
//! behave as in any CSS, and a stored value is closed, so it can't run into
//! the next declaration. Stylo checks a value when it knows the property;
//! that is the parser the printing engine uses, so what it drops, the printer
//! couldn't use. A property only `css_properties` knows (`fill`,
//! `break-inside`) is checked as tokens. Stored values stay as written, apart
//! from comments, runs of whitespace and string quotes: a browser would
//! compute them (`#F00`) and expand shorthands.
//!
//! Stylo's parse can't be interrupted, so blitz-dom never parses this
//! attribute here (see [`hide`]). The only calls to stylo are one declaration
//! at a time, from [`valid`], under [`MAX_CHECKED_VALUE`], with the deadline
//! looked at in between. [`fits`] bounds what the printer's stylo gets.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use blitz_dom::Attribute;
use blitz_traits::net::Url;
use cssparser::{
    Delimiter, ParseError, ParseErrorKind, Parser, ParserInput, ToCss, Token, serialize_identifier,
};
use html5ever::{LocalName, QualName};
use style::context::QuirksMode;
use style::properties::{PropertyId, parse_style_attribute};
use style::stylesheets::{CssRuleType, UrlExtraData};
use surimono_core::Deadline;

use crate::DomError;
use crate::css_properties::CSS_PROPERTIES;

/// Most declarations in one `style` attribute, empty ones included. Every
/// read parses the attribute here and each declaration goes through stylo, so
/// this bounds the declarations such a read keeps, outside the JavaScript heap
/// limit, and, with the deadline checked per declaration, its work.
pub(crate) const MAX_DECLARATIONS: usize = 4096;
/// Most bytes in one `style` attribute or value; big enough for a `data:`
/// image.
pub(crate) const MAX_BYTES: usize = 8 << 20;
/// Most CSS tokens in one `style` attribute, whitespace included. The
/// printer's stylo takes a whole attribute in one call, and its cost follows
/// tokens, not bytes: at the 300 ns a byte of [`MAX_CHECKED_VALUE`], `a,a,…`
/// costs about 20 ms here, where a `data:` image is one token. See [`fits`].
pub(crate) const MAX_TOKENS: usize = 1 << 16;
/// Most list steps stylo may take over one attribute, as [`Budget::declare`]
/// counts them. Stylo's `PropertyDeclarationBlock::push` walks the block for a
/// property it already has, and `all` pushes every longhand, so a block's cost
/// is its repeats times its size: 8,000 custom properties and 8,000
/// `all:inherit;`, 60 KB, took 11 s.
const MAX_PUSH_STEPS: u64 = 1 << 26;
/// Most cells in the strings of one `grid-template-areas`, `grid-template` or
/// `grid` declaration. Stylo tokenizes those strings again and interns each
/// name, about 5 µs each: 150,000 names, 1 MB, took 787 ms. No other string
/// costs more than its bytes. 4,096 is a 64 by 64 grid.
const MAX_GRID_CELLS: usize = 4096;
/// Deepest nesting of brackets in a value, for the recursion here and in
/// stylo's `calc()`. A deeper value is invalid, and [`fits`] refuses an
/// attribute holding one.
const MAX_NESTING: usize = 32;
/// Longest value this file's own check gives to stylo; a longer one (in
/// practice a `data:` image) is checked as tokens only here. A stylo parse
/// can't be interrupted and costs up to about 300 ns a byte in a release
/// build (`stylo_cost_by_property_and_shape`): 20 ms here, but past the time
/// limit at [`MAX_BYTES`].
const MAX_CHECKED_VALUE: usize = 64 << 10;
/// What a `style` attribute is called while blitz-dom builds its element. No
/// parser or script can give an attribute this name: html5ever turns NUL into
/// U+FFFD, and the DOM refuses it.
const HIDDEN_STYLE: &str = "\0style";

/// `url()` values are kept, not fetched; they need a base to parse.
static BASE: LazyLock<UrlExtraData> = LazyLock::new(|| {
    // A constant, valid absolute URL.
    Url::parse("about:blank")
        .expect("about:blank is a URL")
        .into()
});

/// One `name: value` of a style attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declaration {
    pub name: String,
    pub value: String,
    pub important: bool,
}

/// Why parsing gave up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    /// Over the limits of [`fits`].
    TooLarge,
    /// The script's deadline passed.
    TimeLimit,
}

/// The error for a `style` attribute over the limits.
pub(crate) fn limit_error() -> DomError {
    DomError::StyleLimit {
        declarations: MAX_DECLARATIONS,
        tokens: MAX_TOKENS,
        bytes: MAX_BYTES,
    }
}

/// Renames a `style` attribute so that blitz-dom doesn't see it. blitz-dom goes
/// by the local name, parses the attribute as soon as it sees one, in a call
/// nothing can stop, and keeps the result for a layout that doesn't run here.
pub(crate) fn hide(name: &mut QualName) {
    name.local = LocalName::from(HIDDEN_STYLE);
}

/// Undoes [`hide`].
pub(crate) fn reveal(attrs: &mut [Attribute]) {
    for a in attrs {
        if a.name.local.as_ref() == HIDDEN_STYLE {
            a.name.local = LocalName::from("style");
        }
    }
}

/// Whether `text` may be a `style` attribute: within [`MAX_BYTES`],
/// [`MAX_DECLARATIONS`], [`MAX_TOKENS`], [`MAX_NESTING`], [`MAX_PUSH_STEPS`]
/// and [`MAX_GRID_CELLS`]. Every path that sets the attribute goes through
/// this first, the parser's included. What passes reaches the printer's stylo.
pub(crate) fn fits(text: &str) -> bool {
    size_check(text, Budget::for_attribute()).is_none()
}

/// What is over the limits in `text`, if anything.
fn size_check(text: &str, mut budget: Budget) -> Option<Over> {
    if text.len() > MAX_BYTES {
        return Some(Over::Size);
    }
    let mut input = ParserInput::new(text);
    match spend_tokens(&mut Parser::new(&mut input), &mut budget, 0) {
        Ok(()) => None,
        Err(ParseError {
            kind: ParseErrorKind::Custom(over),
            ..
        }) => Some(over),
        // Only `Over` is raised here, and tokenizing itself can't fail.
        Err(_) => Some(Over::Size),
    }
}

/// How text is over the limits of [`fits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Over {
    /// All but [`MAX_NESTING`].
    Size,
    /// [`MAX_NESTING`].
    Depth,
}

/// What is left to spend on one attribute or value, and how far it has got.
struct Budget {
    tokens: usize,
    /// Declarations are counted, for a whole attribute.
    attribute: bool,
    /// The next token that isn't whitespace or a comment starts a declaration.
    at_start: bool,
    declarations: usize,
    /// Names seen, and how long stylo's list is with one each.
    names: HashSet<String>,
    block: usize,
    push_steps: u64,
    /// The current declaration's strings are grid areas.
    in_grid: bool,
    grid_cells: usize,
}

impl Budget {
    fn for_attribute() -> Self {
        Self {
            tokens: MAX_TOKENS,
            attribute: true,
            at_start: true,
            declarations: 0,
            names: HashSet::new(),
            block: 0,
            push_steps: 0,
            in_grid: false,
            grid_cells: 0,
        }
    }

    /// For the value of `property`, already [`normalized`].
    fn for_value(property: &str) -> Self {
        Self {
            attribute: false,
            in_grid: is_grid_areas(property),
            ..Self::for_attribute()
        }
    }

    /// Charges a token at the top level of an attribute, false if over.
    fn top_level(&mut self, token: &Token<'_>) -> bool {
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => true,
            Token::Semicolon => {
                let empty = self.at_start;
                self.at_start = true;
                self.in_grid = false;
                self.grid_cells = 0;
                !empty || self.count_declaration()
            }
            _ if self.at_start => {
                self.at_start = false;
                let under = self.count_declaration();
                match token {
                    Token::Ident(name) => under && self.declare(normalized(name)),
                    _ => under,
                }
            }
            _ => true,
        }
    }

    fn count_declaration(&mut self) -> bool {
        self.declarations += 1;
        self.declarations <= MAX_DECLARATIONS
    }

    /// Charges a declaration of `name` for stylo's pushes. A push finds a
    /// property that is already there by walking the list, then closes the
    /// gap: at most the list's length in steps, each a few ns. A custom
    /// property is new unless its name was seen; any other may overlap a
    /// shorthand, so is taken to be there. False if over.
    fn declare(&mut self, name: String) -> bool {
        self.in_grid = is_grid_areas(&name);
        let pushes = longhands(&name);
        let custom = is_custom(&name);
        let first = self.names.insert(name);
        if first {
            self.block += pushes;
        }
        if !first || !custom {
            self.push_steps = self
                .push_steps
                .saturating_add(pushes.saturating_mul(self.block) as u64);
        }
        self.push_steps <= MAX_PUSH_STEPS
    }

    /// Charges a string for the cells stylo would split it into, as
    /// `TemplateAreasTokenizer` does; false if over.
    fn string(&mut self, text: &str) -> bool {
        if self.in_grid {
            let cells = text
                .split(|c: char| c.is_ascii_whitespace() || c == '.')
                .filter(|cell| !cell.is_empty())
                .count();
            self.grid_cells = self.grid_cells.saturating_add(cells);
        }
        self.grid_cells <= MAX_GRID_CELLS
    }
}

/// Whether strings in a value of `property` are grid areas.
fn is_grid_areas(property: &str) -> bool {
    matches!(property, "grid-template-areas" | "grid-template" | "grid")
}

/// How many longhands stylo pushes for a declaration of `property`.
fn longhands(property: &str) -> usize {
    PropertyId::parse_enabled_for_all_content(property)
        .ok()
        .and_then(|id| id.as_shorthand().ok())
        .map_or(1, |shorthand| shorthand.longhands().count())
}

/// Takes the tokens of `input` from `budget`, entering blocks, whose contents
/// the parser would otherwise skip. Fails when over it, or too deep.
fn spend_tokens<'i>(
    input: &mut Parser<'i, '_>,
    budget: &mut Budget,
    depth: usize,
) -> Result<(), ParseError<'i, Over>> {
    while let Ok(token) = input.next_including_whitespace_and_comments() {
        let block = matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        );
        let mut under = budget.tokens > 0;
        if depth == 0 && budget.attribute {
            under = budget.top_level(token) && under;
        }
        if let Token::QuotedString(text) = token {
            under = budget.string(text) && under;
        }
        if !under {
            return Err(input.new_custom_error(Over::Size));
        }
        if block && depth == MAX_NESTING {
            return Err(input.new_custom_error(Over::Depth));
        }
        budget.tokens -= 1;
        if block {
            input.parse_nested_block(|p| spend_tokens(p, budget, depth + 1))?;
        }
    }
    Ok(())
}

fn is_custom(property: &str) -> bool {
    property.starts_with("--")
}

fn in_table(property: &str) -> Option<&'static str> {
    let at = CSS_PROPERTIES
        .binary_search_by(|(name, _)| (*name).cmp(property))
        .ok()?;
    Some(CSS_PROPERTIES[at].1)
}

fn known_to_stylo(property: &str) -> bool {
    PropertyId::parse_enabled_for_all_content(property).is_ok()
}

/// Lower-cases `property` and follows a legacy alias, unless it is a custom
/// property.
pub(crate) fn normalized(property: &str) -> String {
    if is_custom(property) {
        return property.to_owned();
    }
    let lower = property.to_ascii_lowercase();
    match in_table(&lower) {
        Some(alias) if !alias.is_empty() => alias.to_owned(),
        _ => lower,
    }
}

/// Whether `property`, already [`normalized`], is a supported CSS property: a
/// custom property, or one in webref's list or stylo's. The custom name is
/// any text, as in Chrome; [`serialize`] escapes it.
pub(crate) fn supported(property: &str) -> bool {
    if is_custom(property) {
        property.len() > 2
    } else {
        in_table(property).is_some() || known_to_stylo(property)
    }
}

/// The CSS property for a `CSSStyleProperties` attribute name (CSSOM §6.7):
/// `cssFloat`, the camel-cased `fontSize`, the webkit-cased `webkitTransform`
/// or the dashed `font-size`. `None` if that isn't one, custom properties
/// included.
pub(crate) fn property_for_attribute(attribute: &str) -> Option<String> {
    let dashed = if attribute == "cssFloat" {
        "float".to_owned()
    } else if attribute.contains('-') {
        if attribute.chars().any(|c| c.is_ascii_uppercase()) {
            return None;
        }
        attribute.to_owned()
    } else {
        let mut out = String::with_capacity(attribute.len() + 2);
        if attribute.starts_with("webkit") {
            out.push('-');
        }
        for c in attribute.chars() {
            if c.is_ascii_uppercase() {
                out.push('-');
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    };
    let dashed = normalized(&dashed);
    (!is_custom(&dashed) && supported(&dashed)).then_some(dashed)
}

type Failed<'i> = ParseError<'i, ()>;

/// Writes the rest of `input`, a `<declaration-value>` (CSS Syntax §8.2), to
/// `out`, and says whether it ended in `!important`. Fails on what the
/// grammar forbids: an unmatched closing bracket, a bad string or URL, and a
/// top-level `;` or `!`.
///
/// A custom property keeps its whitespace and comments (css-variables §3).
/// Any other value loses comments, and whitespace collapses. Each token is
/// copied from the source unless that could leave it open: a string or URL
/// that ended with the input, or a trailing backslash, are written closed.
fn value_text<'i>(
    input: &mut Parser<'i, '_>,
    out: &mut String,
    custom: bool,
    depth: usize,
) -> Result<bool, Failed<'i>> {
    let mut gap = String::new();
    loop {
        let start = input.position();
        let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
            return Ok(false);
        };
        match &token {
            Token::WhiteSpace(_) | Token::Comment(_) if !custom => gap = " ".to_owned(),
            Token::WhiteSpace(_) | Token::Comment(_) => {
                // Writing to a String can't fail.
                let _ = token.to_css(&mut gap);
            }
            Token::Delim('!') if depth == 0 => {
                input.expect_ident_matching("important")?;
                input.expect_exhausted()?;
                return Ok(true);
            }
            Token::Semicolon if depth == 0 => return Err(input.new_custom_error(())),
            Token::BadString(_)
            | Token::BadUrl(_)
            | Token::CloseParenthesis
            | Token::CloseSquareBracket
            | Token::CloseCurlyBracket
            | Token::Delim('\\') => return Err(input.new_custom_error(())),
            _ => {
                if !out.is_empty() {
                    out.push_str(&gap);
                }
                gap.clear();
                let closer = match &token {
                    Token::Function(_) | Token::ParenthesisBlock => Some(')'),
                    Token::SquareBracketBlock => Some(']'),
                    Token::CurlyBracketBlock => Some('}'),
                    _ => None,
                };
                let raw = input.slice_from(start);
                if let Some(closer) = closer {
                    if depth == MAX_NESTING {
                        return Err(input.new_custom_error(()));
                    }
                    out.push_str(raw);
                    input.parse_nested_block(|p| value_text(p, out, custom, depth + 1))?;
                    out.push(closer);
                } else if raw.ends_with('\\')
                    || matches!(token, Token::QuotedString(_) | Token::UnquotedUrl(_))
                {
                    let _ = token.to_css(out);
                } else {
                    out.push_str(raw);
                }
            }
        }
    }
}

/// Whether a value that tokenized is valid for `property`.
fn valid(property: &str, value: &str, quirks: QuirksMode) -> bool {
    if is_custom(property) {
        return true;
    }
    if value.is_empty() {
        return false;
    }
    if value.len() > MAX_CHECKED_VALUE || !known_to_stylo(property) {
        return true;
    }
    let text = format!("{property}: {value}");
    !parse_style_attribute(&text, &BASE, None, quirks, CssRuleType::Style)
        .declarations()
        .is_empty()
}

/// CSSOM "parse a CSS value" for `property`, already [`normalized`]: the text
/// to store, `None` if the value is invalid.
pub(crate) fn parse_value(
    property: &str,
    value: &str,
    quirks: QuirksMode,
) -> Result<Option<String>, Stop> {
    match size_check(value, Budget::for_value(property)) {
        Some(Over::Size) => return Err(Stop::TooLarge),
        Some(Over::Depth) => return Ok(None),
        None => {}
    }
    let mut input = ParserInput::new(value);
    let mut text = String::new();
    let parsed = value_text(
        &mut Parser::new(&mut input),
        &mut text,
        is_custom(property),
        0,
    );
    let ok = matches!(parsed, Ok(false)) && valid(property, &text, quirks);
    Ok(ok.then_some(text))
}

/// One declaration of a style attribute, up to its `;`.
fn declaration<'i>(
    input: &mut Parser<'i, '_>,
    quirks: QuirksMode,
) -> Result<Declaration, Failed<'i>> {
    let name = normalized(input.expect_ident()?);
    input.expect_colon()?;
    let mut value = String::new();
    let important = value_text(input, &mut value, is_custom(&name), 0)?;
    if supported(&name) && valid(&name, &value, quirks) {
        Ok(Declaration {
            name,
            value,
            important,
        })
    } else {
        Err(input.new_custom_error(()))
    }
}

/// Parses a style attribute (CSS Style Attributes §3; CSS Syntax "parse a
/// block's contents"), skipping what is invalid. For a name given twice, an
/// `!important` declaration outlasts a later normal one (css-cascade §6.5);
/// otherwise the later one wins and takes the later position.
///
/// Linear in `text`: a map finds the earlier declaration.
pub(crate) fn parse(
    text: &str,
    quirks: QuirksMode,
    deadline: Option<&Deadline>,
) -> Result<Vec<Declaration>, Stop> {
    if text.len() > MAX_BYTES {
        return Err(Stop::TooLarge);
    }
    let mut input = ParserInput::new(text);
    let mut parser = Parser::new(&mut input);
    let mut slots: Vec<Option<Declaration>> = Vec::new();
    // Only looked up in, so its order can't reach the output.
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut seen = 0;
    while !parser.is_exhausted() {
        seen += 1;
        if seen > MAX_DECLARATIONS {
            return Err(Stop::TooLarge);
        }
        if deadline.is_some_and(Deadline::passed) {
            return Err(Stop::TimeLimit);
        }
        let Ok(d) = parser.parse_until_after(Delimiter::Semicolon, |p| declaration(p, quirks))
        else {
            continue;
        };
        if let Some(&old) = at.get(&d.name) {
            if slots[old].as_ref().is_some_and(|o| o.important) && !d.important {
                continue;
            }
            slots[old] = None;
        }
        at.insert(d.name.clone(), slots.len());
        slots.push(Some(d));
    }
    Ok(slots.into_iter().flatten().collect())
}

/// CSSOM "set a CSS declaration": in place if already there.
pub(crate) fn set(list: &mut Vec<Declaration>, name: String, value: String, important: bool) {
    match list.iter_mut().find(|d| d.name == name) {
        Some(d) => {
            d.value = value;
            d.important = important;
        }
        None => list.push(Declaration {
            name,
            value,
            important,
        }),
    }
}

/// `name`'s value, empty if unset.
pub(crate) fn value(list: &[Declaration], name: &str) -> String {
    list.iter()
        .find(|d| d.name == name)
        .map(|d| d.value.clone())
        .unwrap_or_default()
}

/// CSSOM "serialize a CSS declaration block".
pub(crate) fn serialize(list: &[Declaration]) -> String {
    let mut out = String::new();
    for d in list {
        if !out.is_empty() {
            out.push(' ');
        }
        // Writing to a String can't fail.
        let _ = serialize_identifier(&d.name, &mut out);
        out.push_str(": ");
        out.push_str(&d.value);
        if d.important {
            out.push_str(" !important");
        }
        out.push(';');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_QUIRKS: QuirksMode = QuirksMode::NoQuirks;

    fn parsed(text: &str) -> Vec<Declaration> {
        parse(text, NO_QUIRKS, None).unwrap()
    }

    /// `cssText` after the attribute is `text`.
    fn css_text(text: &str) -> String {
        serialize(&parsed(text))
    }

    /// Chrome 153.0.8010.12: `cssText` of an element with this `style`
    /// attribute, where the values are ones we keep as written.
    #[test]
    fn parses_as_chrome_does() {
        for (text, chrome) in [
            (
                "color: red; /* a;b */ width: 1px",
                "color: red; width: 1px;",
            ),
            ("/* x */ color: red", "color: red;"),
            ("color /* c */ : red", "color: red;"),
            ("color: /* c */ red", "color: red;"),
            ("color: red /* c */;", "color: red;"),
            (
                "color: red !important; color: blue",
                "color: red !important;",
            ),
            (
                "color: red; width: 1px; color: blue",
                "width: 1px; color: blue;",
            ),
            (
                "color: red !important; width: 1px; color: blue !important",
                "width: 1px; color: blue !important;",
            ),
            ("color: red; color: notacolor", "color: red;"),
            ("color: notacolor; width: 10px", "width: 10px;"),
            ("color: red ! important", "color: red !important;"),
            ("color: red !IMPORTANT", "color: red !important;"),
            ("color: red !important x; width: 1px", "width: 1px;"),
            ("color: red !importantx", ""),
            ("color: !important", ""),
            ("width: 1px; ;;; height: 2px;;", "width: 1px; height: 2px;"),
            ("a{b} width: 1px", ""),
            ("a{b}; width: 1px", "width: 1px;"),
            ("color: {red}; width: 1px", "width: 1px;"),
            ("width: 1px; }; height: 2px", "width: 1px; height: 2px;"),
            ("width: 1px; ); height: 2px", "width: 1px; height: 2px;"),
            ("(; width: 1px", ""),
            ("color: red; (; ); width: 1px", "color: red; width: 1px;"),
            (
                "content: 'x;y'; width: 1px",
                "content: \"x;y\"; width: 1px;",
            ),
            ("content: 'abc", "content: \"abc\";"),
            ("content: \"a\\", "content: \"a\";"),
            ("content: 'a\nb'; width: 1px", ""),
            ("background: url(a b); width: 1px", "width: 1px;"),
            ("--x: 1; --X: 2", "--x: 1; --X: 2;"),
            ("--x:; width: 1px", "--x: ; width: 1px;"),
            ("--x:   a   b  ; width: 1px", "--x: a   b; width: 1px;"),
            ("--x: {a;b}; width: 1px", "--x: {a;b}; width: 1px;"),
            ("--x: a/*c*/b", "--x: a/*c*/b;"),
            ("--x: (a;b); width:1px", "--x: (a;b); width: 1px;"),
            ("--x: a!b; width:1px", "width: 1px;"),
            ("--x: 1 !important", "--x: 1 !important;"),
            ("--: 1; width: 1px", "width: 1px;"),
            ("--a\\:b: 1; width: 1px", "--a\\:b: 1; width: 1px;"),
            ("co\\6cor: red", "color: red;"),
            (
                "color: var(--x); width: var(--y, 1px)",
                "color: var(--x); width: var(--y, 1px);",
            ),
            ("width: 1px; color: inherit", "width: 1px; color: inherit;"),
            ("foo: bar; color: red", "color: red;"),
            ("@media print { color: red }; width: 1px", "width: 1px;"),
            ("color: red; @foo; width: 1px", "color: red; width: 1px;"),
            (
                "color: red; @foo bar; width: 1px",
                "color: red; width: 1px;",
            ),
            ("width: 10", ""),
            ("color: ff0000", ""),
            ("<!-- color: red", ""),
            ("color: red; -->; width: 1px", "color: red; width: 1px;"),
        ] {
            assert_eq!(css_text(text), chrome, "{text:?}");
        }
    }

    /// CSSOM keeps source order. Chrome 153 sometimes moves important
    /// declarations after normal ones (it gives `width: 1px; color: blue
    /// !important;` here, but keeps `color: red !important; width: 1px;`);
    /// the spec wins for DOM behaviour (docs/COMPAT.md).
    #[test]
    fn important_keeps_source_order() {
        assert_eq!(
            css_text("color: red; color: blue !important; width: 1px"),
            "color: blue !important; width: 1px;"
        );
    }

    #[test]
    fn a_value_stays_closed() {
        for (text, want) in [
            ("background-image: url(a", "background-image: url(a);"),
            ("font-family: a\\", "font-family: a\u{fffd};"),
            ("margin: 1px/**/2px", "margin: 1px 2px;"),
            ("width: calc(1px + (2px", "width: calc(1px + (2px));"),
            ("--x: '", "--x: \"\";"),
            ("--x: a /* b", "--x: a;"),
            // Chrome 153 gives `a`: it computes the family name, and stored
            // values stay as written, closed.
            ("font-family: 'a", "font-family: \"a\";"),
        ] {
            let got = css_text(text);
            assert_eq!(got, want, "{text:?}");
            // What we write parses back to itself, with a declaration after it.
            let more = format!("{got} top: 0");
            assert_eq!(css_text(&more), format!("{want} top: 0;"), "{text:?}");
        }
    }

    /// CSSOM "parse a CSS value"; results from Chrome 153.0.8010.12.
    #[test]
    fn values() {
        for (name, value, chrome) in [
            ("color", "red; display: none", None),
            ("color", "red !important", None),
            ("color", "red;", None),
            ("color", "red/* c */", Some("red")),
            ("color", " red ", Some("red")),
            ("color", " ", None),
            ("color", "notacolor", None),
            ("color", "red)", None),
            ("--x", "1;", None),
            ("--x", "a;b", None),
            ("--x", "{a;b}", Some("{a;b}")),
            ("--x", " ", Some("")),
            ("--x", "a!b", None),
            ("--x", ")", None),
            ("content", "'a", Some("\"a\"")),
        ] {
            let got = parse_value(name, value, NO_QUIRKS).unwrap();
            assert_eq!(got.as_deref(), chrome, "{name}: {value:?}");
        }
    }

    #[test]
    fn hostile_text_does_not_panic() {
        for s in [
            "\\",
            "'",
            "(((",
            "a:b\\",
            "é:é!important",
            ":",
            ";;;",
            "x:日本語important",
            "/*",
            "--x:\\",
            "--x:/*",
            "color:\0",
            "\0:\0",
        ] {
            let once = css_text(s);
            assert_eq!(css_text(&once), once, "{s:?}");
        }
    }

    #[test]
    fn what_fits_counts_tokens_not_bytes() {
        let list = |tokens: usize| format!("font-family: {}a", "a,".repeat(tokens / 2));
        // `font-family`, `:`, a space, then the list.
        assert!(fits(&list(MAX_TOKENS - 4)));
        assert!(!fits(&list(MAX_TOKENS)));
        // Tokens inside a block count.
        assert!(!fits(&format!("x: f({})", "a,".repeat(MAX_TOKENS))));
        assert!(!fits(&format!("x: [{}]", "a ".repeat(MAX_TOKENS))));
        // One token however long, in or out of a block, apart from grid areas
        // (`what_fits_counts_grid_cells`).
        let data = "A".repeat(MAX_BYTES - 64);
        assert!(fits(&format!(
            "background: url(data:image/png;base64,{data})"
        )));
        assert!(fits(&format!("content: var(--x, '{data}')")));
        assert!(fits(&format!("/* {data} */ color: red")));
        assert!(!fits(&"x".repeat(MAX_BYTES + 1)));
    }

    #[test]
    fn what_fits_counts_declarations() {
        assert!(fits(&"a:b;".repeat(MAX_DECLARATIONS)));
        assert!(!fits(&format!("{}a:b", "a:b;".repeat(MAX_DECLARATIONS))));
        assert!(fits(&format!(
            "{}  /* c */ ",
            "a:b;".repeat(MAX_DECLARATIONS)
        )));
        // As `parse` counts: it gives up on what `fits` refuses, and only then.
        for text in [
            ";".repeat(MAX_DECLARATIONS),
            ";".repeat(MAX_DECLARATIONS + 1),
            format!("a:b;{}", " ; ".repeat(MAX_DECLARATIONS - 1)),
            format!("a:b;{}", " ; ".repeat(MAX_DECLARATIONS)),
            format!("--x:(;);{}", "a:b;".repeat(MAX_DECLARATIONS - 1)),
            format!("--x:(;);{}", "a:b;".repeat(MAX_DECLARATIONS)),
            format!("--x:{{;}};{}", "a:b;".repeat(MAX_DECLARATIONS)),
        ] {
            let read = parse(&text, NO_QUIRKS, None);
            assert_eq!(fits(&text), read.is_ok(), "{}", &text[..16]);
        }
    }

    #[test]
    fn what_fits_counts_repeats_times_the_block() {
        let all = "all:inherit;";
        let pushes = longhands("all");
        assert!(pushes > 100);
        let customs: String = (0..1024).map(|i| format!("--a{i}:;")).collect();
        let fitting = MAX_PUSH_STEPS as usize / pushes / (1024 + pushes);
        assert!(fits(&(customs.clone() + &all.repeat(fitting))));
        assert!(!fits(&(customs.clone() + &all.repeat(fitting + 1))));
        // However it is spelled.
        for spelling in ["ALL:inherit;", "\\61ll:inherit;", "a\\6c\\6c:inherit;"] {
            assert!(!fits(&(customs.clone() + &spelling.repeat(fitting + 1))));
        }
        // Any shorthand, by its longhands.
        assert_eq!(longhands("margin"), 4);
        let long = "font:inherit;";
        let pushes = longhands("font");
        let block = 1024 + pushes;
        let fitting = MAX_PUSH_STEPS as usize / pushes / block;
        if fitting + 1024 < MAX_DECLARATIONS {
            assert!(!fits(&(customs.clone() + &long.repeat(fitting + 1))));
        }
        // New custom properties aren't repeats.
        let new: String = (0..MAX_DECLARATIONS).map(|i| format!("--a{i}:;")).collect();
        assert!(fits(&new));
    }

    #[test]
    fn what_fits_counts_grid_cells() {
        let cells = |n: usize| (0..n).map(|i| format!("n{i} ")).collect::<String>();
        for property in ["grid-template-areas", "grid-template", "grid", "GRID"] {
            assert!(fits(&format!("{property}: '{}'", cells(MAX_GRID_CELLS))));
            assert!(!fits(&format!(
                "{property}: '{}'",
                cells(MAX_GRID_CELLS + 1)
            )));
            assert!(!fits(&format!(
                "{property}: 'a' '{}'",
                cells(MAX_GRID_CELLS)
            )));
            assert!(!fits(&format!(
                "{property}: '{}'",
                "a.".repeat(MAX_GRID_CELLS + 1)
            )));
        }
        // Per declaration.
        let at_cap = format!("grid-template-areas: '{}'", cells(MAX_GRID_CELLS));
        assert!(fits(&format!("{at_cap}; {at_cap}")));
        // Other strings are one token.
        let over = cells(MAX_GRID_CELLS + 1);
        assert!(fits(&format!("content: '{over}'; --x: '{over}'")));
        // As a value too, and a list of semicolons stays merely invalid.
        let got = parse_value("grid-template-areas", &format!("'{over}'"), NO_QUIRKS);
        assert_eq!(got, Err(Stop::TooLarge));
        let semicolons = ";".repeat(MAX_DECLARATIONS + 1);
        assert_eq!(parse_value("color", &semicolons, NO_QUIRKS), Ok(None));
    }

    #[test]
    fn what_fits_is_bounded_in_depth() {
        let at = |n: usize| format!("--x: {}1{}", "a(".repeat(n), ")".repeat(n));
        assert!(fits(&at(MAX_NESTING)));
        assert!(!fits(&at(MAX_NESTING + 1)));
        for open in ["(", "[", "{", "f("] {
            assert!(fits(&open.repeat(MAX_NESTING)), "{open}");
            assert!(!fits(&open.repeat(MAX_NESTING + 1)), "{open}");
        }
    }

    #[test]
    fn nesting_is_bounded() {
        let at = |n: usize| format!("width: {}1px{}", "calc(".repeat(n), ")".repeat(n));
        assert_eq!(parsed(&at(MAX_NESTING)).len(), 1);
        assert!(parsed(&at(MAX_NESTING + 1)).is_empty());
        // Far past the limit, closed or not, on a small stack.
        std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || {
                assert_eq!(parsed(&at(MAX_NESTING)).len(), 1);
                assert!(parsed(&format!("width: {}", "(".repeat(1 << 20))).is_empty());
                assert!(parsed(&at(1 << 18)).is_empty());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn size_limits() {
        let at_limit = "a:b;".repeat(MAX_DECLARATIONS);
        assert_eq!(parse(&at_limit, NO_QUIRKS, None), Ok(Vec::new()));
        let over = format!("{at_limit}a:b");
        assert_eq!(parse(&over, NO_QUIRKS, None), Err(Stop::TooLarge));
        let long = format!("--x: '{}'", "a".repeat(MAX_BYTES));
        assert_eq!(parse(&long, NO_QUIRKS, None), Err(Stop::TooLarge));
        assert_eq!(parse_value("--x", &long, NO_QUIRKS), Err(Stop::TooLarge));
        // Longer than stylo gets: tokens are still checked.
        let long = format!("url({})", "a".repeat(MAX_CHECKED_VALUE));
        let got = parse_value("background-image", &long, NO_QUIRKS).unwrap();
        assert_eq!(got.as_deref(), Some(long.as_str()));
        let bad = format!("{long};");
        assert_eq!(parse_value("background-image", &bad, NO_QUIRKS), Ok(None));
    }

    /// One pass over the text, so a list of duplicates near the cap costs
    /// what any other list does.
    #[test]
    fn duplicates_near_the_cap() {
        let text: String = (0..MAX_DECLARATIONS - 1)
            .map(|i| format!("--a{}: {i};", i % 2))
            .collect();
        let got = serialize(&parsed(&text));
        // 4,095 declarations, numbered from 0: the last of each name survive.
        assert_eq!(got, "--a1: 4093; --a0: 4094;");
    }

    #[test]
    fn a_passed_deadline_stops_the_parse() {
        // A dropped sandbox's deadline counts as passed.
        let deadline = surimono_core::Sandbox::new(surimono_core::Limits::default())
            .unwrap()
            .deadline();
        assert_eq!(
            parse("color: red", NO_QUIRKS, Some(&deadline)),
            Err(Stop::TimeLimit)
        );
    }

    /// Every value of every shape, at every size `fits` lets through, against
    /// every property stylo knows: how long stylo takes, worst first. Run on a
    /// release build; a new cost shape shows at the top.
    /// `--release --lib -- --ignored --nocapture stylo_cost`
    #[test]
    #[ignore = "a survey, for a release build"]
    fn stylo_cost_by_property_and_shape() {
        type Shape = (
            &'static str,
            &'static str,
            fn(usize) -> String,
            &'static str,
            &'static str,
        );
        let shapes: &[Shape] = &[
            ("long string", "\"", |_| "a".into(), "", "\""),
            ("string of names", "\"", |i| format!("n{i}"), " ", "\""),
            (
                "string of long names",
                "\"",
                |i| format!("{i:01024}"),
                " ",
                "\"",
            ),
            ("string of same", "\"", |_| "a".into(), " ", "\""),
            ("string of dots", "\"", |_| ".".into(), " ", "\""),
            ("string of escapes", "\"", |_| "\\61 ".into(), "", "\""),
            ("strings", "", |i| format!("\"n{i} m{i}\""), " ", ""),
            ("strings, commas", "", |i| format!("\"n{i}\""), ",", ""),
            ("long ident", "", |_| "a".into(), "", ""),
            ("idents", "", |_| "a".into(), " ", ""),
            ("unique idents", "", |i| format!("n{i}"), " ", ""),
            ("ident commas", "", |_| "a".into(), ",", ""),
            ("unique commas", "", |i| format!("n{i}"), ",", ""),
            ("ident slash", "", |_| "a".into(), "/", ""),
            ("numbers", "", |_| "1".into(), " ", ""),
            ("number commas", "", |_| "1".into(), ",", ""),
            ("lengths", "", |_| "1px".into(), " ", ""),
            ("length commas", "", |_| "1px".into(), ",", ""),
            ("percent commas", "", |_| "1%".into(), ",", ""),
            ("times", "", |_| "1s".into(), ",", ""),
            ("colors", "", |_| "#fff".into(), " ", ""),
            ("color commas", "", |_| "red".into(), ",", ""),
            ("name number", "", |i| format!("n{i} 1"), " ", ""),
            ("urls", "", |_| "url(a)".into(), " ", ""),
            ("url commas", "", |_| "url(a)".into(), ",", ""),
            ("functions", "", |_| "f(1)".into(), " ", ""),
            ("function commas", "", |_| "f(1)".into(), ",", ""),
            ("transforms", "", |_| "translate(1px)".into(), " ", ""),
            ("shadows", "", |_| "1px 1px red".into(), ",", ""),
            (
                "gradient stops",
                "linear-gradient(",
                |_| "red".into(),
                ",",
                ")",
            ),
            (
                "gradients",
                "",
                |_| "linear-gradient(red,red)".into(),
                ",",
                "",
            ),
            ("calc sum", "calc(", |_| "1px".into(), " + ", ")"),
            ("calc product", "calc(1px", |_| "*1".into(), "", ")"),
            ("min list", "min(", |_| "1px".into(), ",", ")"),
            ("calcs", "", |_| "calc(1px + 1%)".into(), " ", ""),
            ("line names", "", |i| format!("[n{i}] 1px"), " ", ""),
            ("line name list", "[", |i| format!("n{i}"), " ", "]"),
            ("repeat", "repeat(1000,", |_| "1px".into(), " ", ")"),
            ("repeats", "", |_| "repeat(1000,1px)".into(), " ", ""),
            ("var", "", |_| "var(--x)".into(), " ", ""),
            ("var fallback", "", |_| "var(--x,1px)".into(), ",", ""),
            ("attr", "", |_| "attr(a)".into(), " ", ""),
            ("counters", "", |_| "counter(a)".into(), " ", ""),
            ("unicode ranges", "", |_| "U+0-7F".into(), ",", ""),
            ("beziers", "", |_| "cubic-bezier(0,0,1,1)".into(), ",", ""),
            ("feature tags", "", |_| "\"liga\" 1".into(), ",", ""),
            ("comments", "", |_| "/**/".into(), "", ""),
            ("whitespace", "a", |_| " ".into(), "", ""),
            ("block", "(", |_| "a".into(), " ", ")"),
        ];
        let mut rows: Vec<(f64, usize, &str, &str)> = Vec::new();
        let properties: Vec<&str> = CSS_PROPERTIES
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| known_to_stylo(name))
            .collect();
        for (shape, open, item, sep, close) in shapes {
            // The sizes that fit, for the longest property name.
            let mut sized = Vec::new();
            let (mut body, mut i, mut size) = (String::new(), 0, 1 << 12);
            loop {
                while body.len() < size {
                    body.push_str(&item(i));
                    body.push_str(sep);
                    i += 1;
                }
                if !fits(&format!("mask-border-outset: {open}{body}{close}")) {
                    break;
                }
                sized.push(body.clone());
                if size == MAX_BYTES - 1024 {
                    break;
                }
                size = (size * 4).min(MAX_BYTES - 1024);
            }
            for property in &properties {
                let mut worst = (0.0_f64, 0);
                for body in &sized {
                    let whole = format!("{property}: {open}{body}{close}");
                    if !fits(&whole) {
                        break;
                    }
                    let started = std::time::Instant::now();
                    let block =
                        parse_style_attribute(&whole, &BASE, None, NO_QUIRKS, CssRuleType::Style);
                    let took = started.elapsed().as_secs_f64() * 1e3;
                    std::hint::black_box(block);
                    if took > worst.0 {
                        worst = (took, whole.len());
                    }
                }
                rows.push((worst.0, worst.1, property, shape));
            }
        }
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("{} cases", rows.len());
        for (ms, bytes, property, shape) in rows.iter().take(60) {
            println!("{ms:9.1} ms {:8} KB  {property}: {shape}", bytes >> 10);
        }
    }

    /// Blocks at [`MAX_PUSH_STEPS`] and [`MAX_DECLARATIONS`]: how long stylo
    /// takes over the costliest that [`fits`] lets through.
    #[test]
    #[ignore = "a survey, for a release build"]
    fn stylo_cost_of_repeats() {
        let customs = |n: usize| (0..n).map(|i| format!("--a{i}:;")).collect::<String>();
        let mut rows: Vec<(f64, String)> = Vec::new();
        let mut try_it = |label: String, text: String| {
            if !fits(&text) {
                return;
            }
            let started = std::time::Instant::now();
            let block = parse_style_attribute(&text, &BASE, None, NO_QUIRKS, CssRuleType::Style);
            let took = started.elapsed().as_secs_f64() * 1e3;
            std::hint::black_box(block);
            rows.push((took, format!("{label}, {} KB", text.len() >> 10)));
        };
        for n in [16, 64, 128, 256, 512, 1024, 2048, 4000] {
            for m in [16, 32, 64, 128, 256, 512, 1024, 2048, 4000] {
                if n + m > MAX_DECLARATIONS {
                    continue;
                }
                let block = customs(n);
                for (name, one) in [
                    ("all", "all:inherit;"),
                    ("all var", "all:var(--x);"),
                    ("font", "font:inherit;"),
                    ("background", "background:red;"),
                    ("margin", "margin:0;"),
                    ("color", "color:red;"),
                ] {
                    try_it(
                        format!("{n} custom, {m} {name}"),
                        block.clone() + &one.repeat(m),
                    );
                    // Over and over, so that each goes in front of the other.
                    let round: String = block.clone() + one;
                    try_it(
                        format!("{m} rounds of {n} custom and {name}"),
                        round.repeat(m),
                    );
                }
            }
        }
        try_it("4,000 repeats of 2".into(), "--a:;--b:;".repeat(2000));
        try_it("2,048 custom twice".into(), customs(2048).repeat(2));
        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (ms, label) in rows.iter().take(25) {
            println!("{ms:9.1} ms  {label}");
        }
    }

    #[test]
    fn attribute_names() {
        // Chrome 153.0.8010.12: `name in style`, and the property a value
        // set through it reaches.
        for (attr, property) in [
            ("fontSize", Some("font-size")),
            ("cssFloat", Some("float")),
            ("float", Some("float")),
            ("font-size", Some("font-size")),
            ("color", Some("color")),
            // Not stylo's, but properties: the printer's SVG code reads them.
            ("fill", Some("fill")),
            ("strokeWidth", Some("stroke-width")),
            ("breakInside", Some("break-inside")),
            // Legacy aliases.
            ("webkitTransform", Some("transform")),
            ("WebkitTransform", Some("transform")),
            ("-webkit-transform", Some("transform")),
            ("wordWrap", Some("overflow-wrap")),
            ("notAProperty", None),
            ("length", None),
            ("--x", None),
            ("Font-Size", None),
            ("FontSize", None),
            ("font_size", None),
            ("Color", None),
        ] {
            assert_eq!(property_for_attribute(attr).as_deref(), property, "{attr}");
        }
    }

    #[test]
    fn property_table_is_sorted_and_stylo_agrees_on_common_names() {
        assert!(CSS_PROPERTIES.windows(2).all(|w| w[0].0 < w[1].0));
        for name in ["color", "font-size", "margin", "fill", "break-inside"] {
            assert!(in_table(name).is_some(), "{name}");
        }
        for (name, alias) in CSS_PROPERTIES {
            assert!(alias.is_empty() || in_table(alias).is_some(), "{name}");
        }
    }
}
