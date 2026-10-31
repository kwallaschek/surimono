//! CSS rewriting for SVG `style` attributes and presentation attributes.
//! (`<style>` elements are removed whole; see `sanitize_svg`.)
//!
//! Everything goes through one CSS Syntax Level 3 tokenizer ([`tokenizer`]),
//! after the spec's preprocessing (CR, CR LF and FF become LF), so this module
//! and the browser agree on every string, comment, `url(...)`, bad url and
//! block. On those tokens:
//!
//! - each `url(...)` is rewritten: `#id` gets the island prefix,
//!   `data:image/...` stays, anything else (bad urls too) becomes `none`;
//! - `font` / `font-family` get the registered families;
//! - a declaration or attribute value holding a function that can load
//!   without `url(` (`image-set()`, `image()`, `src()`, ...), an at-keyword,
//!   an escape in a name, a bad string or anything left open at the end is
//!   dropped, and so is anything in a `style` attribute that isn't a
//!   declaration (at-rules, nested rules);
//! - optionally, only listed properties are kept (the root `<svg>`).
//!
//! Then the result is tokenized again and checked on its own, with no
//! structure: every url is a fragment or an inline image, and no loading
//! function, at-keyword, escape or bad or unclosed token is left anywhere.
//! CSS that fails this check is dropped as a whole, so a gap in the walker
//! fails closed.

mod tokenizer;

use std::ops::Range;

use tokenizer::{Kind, Token, preprocess, tokenize};

use crate::css_font::FontSpec;
use crate::fonts::FontSet;

/// Nesting deeper than this is rejected rather than walked.
const MAX_DEPTH: usize = 64;

/// What the rewriter needs from the SVG around the CSS.
pub(crate) struct CssContext<'a> {
    pub(crate) fonts: &'a FontSet,
    /// Prefix for `#id` references.
    pub(crate) id_prefix: &'a str,
    /// Quote character for family names (the one the surrounding XML does
    /// not use as attribute delimiter).
    pub(crate) quote: char,
}

/// The outcome of rewriting some CSS.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Rewrite {
    /// The CSS is safe as it is.
    Unchanged,
    /// The CSS to write instead.
    Changed(String),
    /// Drop the attribute.
    Drop,
}

/// Rewrites a declaration list (a `style` attribute). With `allowed`, only
/// declarations of those properties (lower case) are kept.
pub(crate) fn rewrite_declarations(
    css: &str,
    cx: &CssContext<'_>,
    allowed: Option<&[&str]>,
) -> Result<Rewrite, String> {
    let pre = preprocess(css);
    let mut w = Walker::new(&pre, cx);
    w.declarations(allowed)?;
    Ok(w.finish(css))
}

/// Rewrites a presentation attribute's value (`fill="url(#g)"`, `mask`,
/// `cursor`, ...), which the browser parses as a CSS property value.
pub(crate) fn rewrite_value(css: &str, cx: &CssContext<'_>) -> Rewrite {
    let pre = preprocess(css);
    let mut w = Walker::new(&pre, cx);
    let end = w.t.len();
    let ok = w.value_is_safe(0, end) && w.urls(0, end).is_ok();
    if !ok {
        return Rewrite::Drop;
    }
    w.finish(css)
}

/// The value to write for a `font-family`: the registered stack, quoted.
/// `None` leaves the value as it is (CSS-wide keywords, `var()`, no fonts).
pub(crate) fn family_value(fonts: &FontSet, list: &str, quote: char) -> Option<String> {
    let lower = list.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "inherit" | "initial" | "unset" | "revert" | "revert-layer"
    ) || lower.contains("var(")
        || lower.contains("env(")
        || lower.contains("attr(")
    {
        return None;
    }
    let families = fonts.resolve_families(list);
    if families.is_empty() {
        return None;
    }
    let quoted: Vec<String> = families.iter().map(|f| css_string(f, quote)).collect();
    Some(quoted.join(", "))
}

/// A CSS string literal holding `s`.
fn css_string(s: &str, quote: char) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push_str(&format!("\\{:x} ", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Functions that can load a resource without a `url(` token, or that
/// build values from outside the CSS. A declaration, rule or attribute
/// holding one is dropped.
fn is_loading_function(lower: &str) -> bool {
    lower.contains("image") // image(), image-set(), -webkit-image-set()
        || matches!(
            lower,
            "src"
                | "expression"
                | "element"
                | "-moz-element"
                | "attr"
                | "paint"
                | "cross-fade"
                | "-webkit-cross-fade"
        )
}

/// A URL that loads nothing: a fragment or an inline image.
fn is_safe_url(url: &str) -> bool {
    url.trim().starts_with('#') || is_inline_image(url)
}

/// `data:image/...` URLs are self-contained and safe to keep.
pub(crate) fn is_inline_image(url: &str) -> bool {
    let compact: String = url
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_control())
        .take(11)
        .collect();
    compact.eq_ignore_ascii_case("data:image/")
}

/// The final, structure-free check of CSS about to be written: tokenized on
/// its own, it holds no url but fragments and inline images, no loading
/// function, at-keyword, escape in a name, or bad or unclosed token.
fn is_safe(css: &str) -> bool {
    let t = tokenize(css);
    t.iter().enumerate().all(|(i, tok)| {
        if tok.escaped || tok.unclosed {
            return false;
        }
        match tok.kind {
            Kind::BadString | Kind::BadUrl | Kind::AtKeyword | Kind::Delim('\\') => false,
            Kind::Url => is_safe_url(&tok.value),
            Kind::Function if tok.value.eq_ignore_ascii_case("url") => {
                let arg = t[i + 1..]
                    .iter()
                    .find(|t| !matches!(t.kind, Kind::Whitespace | Kind::Comment));
                arg.is_some_and(|a| a.kind == Kind::String && is_safe_url(&a.value))
            }
            Kind::Function => !is_loading_function(&tok.value.to_ascii_lowercase()),
            _ => true,
        }
    })
}

/// The closing token for a block opener.
fn closer(kind: Kind) -> Option<Kind> {
    match kind {
        Kind::OpenCurly => Some(Kind::CloseCurly),
        Kind::OpenSquare => Some(Kind::CloseSquare),
        Kind::OpenParen | Kind::Function => Some(Kind::CloseParen),
        _ => None,
    }
}

/// Walks the tokens of preprocessed CSS and collects edits. Token index
/// ranges `[a, b)` cover the bytes `start(a)..start(b)`, as tokens tile the
/// input.
struct Walker<'a> {
    s: &'a str,
    t: Vec<Token>,
    cx: &'a CssContext<'a>,
    edits: Vec<(Range<usize>, String)>,
}

impl<'a> Walker<'a> {
    fn new(s: &'a str, cx: &'a CssContext<'a>) -> Self {
        Walker {
            s,
            t: tokenize(s),
            cx,
            edits: Vec::new(),
        }
    }

    /// The kind of token `i`, or `None` at or past `lim`.
    fn kind(&self, i: usize, lim: usize) -> Option<Kind> {
        if i < lim {
            self.t.get(i).map(|t| t.kind)
        } else {
            None
        }
    }

    /// Byte offset where token `i` starts (the end of the input past the last).
    fn start(&self, i: usize) -> usize {
        self.t.get(i).map_or(self.s.len(), |t| t.range.start)
    }

    fn skip_trivia(&self, mut i: usize, lim: usize) -> usize {
        while matches!(self.kind(i, lim), Some(Kind::Whitespace | Kind::Comment)) {
            i += 1;
        }
        i
    }

    /// Removes the tokens `[a, b)`.
    fn drop_tokens(&mut self, a: usize, b: usize) {
        let r = self.start(a)..self.start(b);
        if !r.is_empty() {
            self.edits.push((r, String::new()));
        }
    }

    /// Index just past the component value starting at `i` (a token, or a
    /// block or function through its matching closer, §5.4.7-9).
    fn component_end(&self, i: usize, lim: usize) -> Result<usize, String> {
        let Some(first) = self.kind(i, lim).and_then(closer) else {
            return Ok(i + 1);
        };
        let mut stack = vec![first];
        let mut j = i + 1;
        while let Some(k) = self.kind(j, lim) {
            if Some(&k) == stack.last() {
                stack.pop();
                if stack.is_empty() {
                    return Ok(j + 1);
                }
            } else if let Some(c) = closer(k) {
                if stack.len() >= MAX_DEPTH {
                    return Err("CSS nests too deeply".into());
                }
                stack.push(c);
            }
            j += 1;
        }
        Ok(j)
    }

    /// Index of the next top-level `;` or `}` from `j` (or `lim`).
    fn until_semicolon(&self, mut j: usize, lim: usize) -> Result<usize, String> {
        loop {
            match self.kind(j, lim) {
                None | Some(Kind::Semicolon | Kind::CloseCurly) => return Ok(j),
                Some(_) => j = self.component_end(j, lim)?,
            }
        }
    }

    /// A `style` attribute: declarations separated by `;` (§5.4.6). Anything
    /// else (at-rules, nested rules, garbage) is dropped up to the next
    /// top-level `;`; a stray `}` is stepped over.
    fn declarations(&mut self, allowed: Option<&[&str]>) -> Result<(), String> {
        let lim = self.t.len();
        let mut i = 0;
        loop {
            match self.kind(i, lim) {
                None => return Ok(()),
                Some(Kind::Whitespace | Kind::Comment | Kind::Semicolon | Kind::CloseCurly) => {
                    i += 1
                }
                Some(k) => {
                    let end = self.until_semicolon(i, lim)?;
                    let colon = self.skip_trivia(i + 1, lim);
                    let is_declaration =
                        k == Kind::Ident && self.kind(colon, end) == Some(Kind::Colon);
                    let name = self.t[i].value.to_ascii_lowercase();
                    if is_declaration && allowed.is_none_or(|list| list.contains(&name.as_str())) {
                        self.declaration(i, colon + 1, end)?;
                    } else {
                        self.drop_tokens(i, end);
                    }
                    i = end;
                }
            }
        }
    }

    /// True if nothing in `[a, b)` (blocks and functions included) can load
    /// or hide anything: no loading function, `url()` function without a
    /// string, at-keyword, escape, bad string or unclosed token.
    fn value_is_safe(&self, a: usize, b: usize) -> bool {
        (a..b.min(self.t.len())).all(|j| {
            let t = &self.t[j];
            if t.escaped || t.unclosed {
                return false;
            }
            match t.kind {
                Kind::BadString | Kind::AtKeyword | Kind::Delim('\\') => false,
                Kind::Function => {
                    let lower = t.value.to_ascii_lowercase();
                    !is_loading_function(&lower)
                        && (lower != "url"
                            || self.kind(self.skip_trivia(j + 1, b), b) == Some(Kind::String))
                }
                _ => true,
            }
        })
    }

    /// True if `[a, b)` holds a url, a bad url or a `url()` function.
    fn has_url(&self, a: usize, b: usize) -> bool {
        self.t[a..b.min(self.t.len())].iter().any(|t| {
            matches!(t.kind, Kind::Url | Kind::BadUrl)
                || (t.kind == Kind::Function && t.value.eq_ignore_ascii_case("url"))
        })
    }

    /// A declaration: name token `n`, value `[a, b)`.
    fn declaration(&mut self, n: usize, a: usize, b: usize) -> Result<(), String> {
        let name = self.t[n].value.to_ascii_lowercase();
        if self.t[n].escaped || !self.value_is_safe(a, b) {
            self.drop_tokens(n, b);
            return Ok(());
        }
        if name == "font-family" || name == "font" {
            if self.has_url(a, b) {
                self.drop_tokens(n, b);
                return Ok(());
            }
            let value = self.start(a)..self.start(b);
            let core = value_core(self.s, value);
            let core_str = &self.s[core.clone()];
            let family = if name == "font-family" {
                Some(core.clone())
            } else {
                FontSpec::parse_with_family_offset(core_str)
                    .and_then(|(_, off)| off)
                    .map(|off| core.start + off..core.end)
            };
            if let Some(f) = family {
                if let Some(new) = family_value(self.cx.fonts, &self.s[f.clone()], self.cx.quote) {
                    self.edits.push((f, new));
                }
            }
            return Ok(());
        }
        self.urls(a, b)
    }

    /// Rewrites every url in `[a, b)`: `#id` gets the island prefix,
    /// `data:image/...` stays, anything else (bad urls too) becomes `none`.
    fn urls(&mut self, a: usize, b: usize) -> Result<(), String> {
        let mut j = a;
        while j < b {
            let t = &self.t[j];
            let (url, end) = match t.kind {
                Kind::Url => (Some(t.value.clone()), j + 1),
                Kind::BadUrl => (None, j + 1),
                Kind::Function if t.value.eq_ignore_ascii_case("url") => {
                    let arg = self.skip_trivia(j + 1, b);
                    let url = match self.kind(arg, b) {
                        Some(Kind::String) => Some(self.t[arg].value.clone()),
                        _ => None,
                    };
                    (url, self.component_end(j, b)?)
                }
                _ => {
                    j += 1;
                    continue;
                }
            };
            let replacement = match url {
                Some(u) => classify_url(&u, self.cx.id_prefix),
                None => Some("none".to_string()),
            };
            if let Some(mut new) = replacement {
                // Keep a following name from running into the replacement.
                let next = self.s[self.start(end)..].bytes().next();
                if next.is_some_and(|c| {
                    c.is_ascii_alphanumeric()
                        || matches!(c, b'-' | b'_' | b'(' | b'\\')
                        || c >= 0x80
                }) {
                    new.push(' ');
                }
                self.edits.push((self.start(j)..self.start(end), new));
            }
            j = end;
        }
        Ok(())
    }

    /// Applies the edits and checks the result. `original` is the CSS before
    /// preprocessing.
    fn finish(mut self, original: &str) -> Rewrite {
        if self.edits.is_empty() && self.s == original {
            return if is_safe(self.s) {
                Rewrite::Unchanged
            } else {
                Rewrite::Drop
            };
        }
        // Larger edits first at one position; an edit inside a removed range
        // is already gone.
        self.edits
            .sort_by(|(x, _), (y, _)| x.start.cmp(&y.start).then(y.end.cmp(&x.end)));
        let mut out = String::with_capacity(self.s.len() + 64);
        let mut pos = 0;
        for (r, new) in &self.edits {
            if r.start < pos {
                continue;
            }
            out.push_str(&self.s[pos..r.start]);
            out.push_str(new);
            pos = r.end;
        }
        out.push_str(&self.s[pos..]);
        if is_safe(&out) {
            Rewrite::Changed(out)
        } else {
            Rewrite::Drop
        }
    }
}

/// The trimmed value of a declaration without `!important`.
fn value_core(s: &str, r: Range<usize>) -> Range<usize> {
    let v = &s[r.clone()];
    let trimmed_end = v.trim_end().len();
    let mut end = trimmed_end;
    let lower = v[..trimmed_end].to_ascii_lowercase();
    if let Some(stripped) = lower.strip_suffix("important") {
        let t = stripped.trim_end();
        if let Some(t) = t.strip_suffix('!') {
            end = t.trim_end().len();
        }
    }
    let start = v.len() - v.trim_start().len();
    r.start + start.min(end)..r.start + end
}

/// What to do with a URL inside `url()`: `None` keeps it as written.
fn classify_url(url: &str, prefix: &str) -> Option<String> {
    let trimmed = url.trim();
    if let Some(id) = trimmed.strip_prefix('#') {
        let new_id = format!("{prefix}{id}");
        if new_id.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':') || b >= 0x80
        }) {
            return Some(format!("url(#{new_id})"));
        }
        return Some(format!(
            "url(\"#{}\")",
            new_id
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\a ")
        ));
    }
    if is_inline_image(trimmed) {
        return None;
    }
    Some("none".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_cx<R>(f: impl FnOnce(&CssContext<'_>) -> R) -> R {
        let fonts = FontSet::new();
        let cx = CssContext {
            fonts: &fonts,
            id_prefix: "sm0-",
            quote: '\'',
        };
        f(&cx)
    }

    /// The rewritten declarations; `<dropped>` if the attribute goes.
    fn decl(css: &str) -> String {
        decl_only(css, None)
    }

    fn decl_only(css: &str, allowed: Option<&[&str]>) -> String {
        match with_cx(|cx| rewrite_declarations(css, cx, allowed)).expect("ok") {
            Rewrite::Unchanged => css.to_string(),
            Rewrite::Changed(s) => s,
            Rewrite::Drop => "<dropped>".into(),
        }
    }

    fn value(css: &str) -> String {
        match with_cx(|cx| rewrite_value(css, cx)) {
            Rewrite::Unchanged => css.to_string(),
            Rewrite::Changed(s) => s,
            Rewrite::Drop => "<dropped>".into(),
        }
    }

    #[test]
    fn untouched_css_is_unchanged() {
        for css in [
            "fill:#fff;stroke:red",
            "font-size:12px;",
            "",
            "  ",
            "a:b;;;c:d",
            "--x:{a:b}",
            "a:'x'/* c */",
        ] {
            assert_eq!(
                with_cx(|cx| rewrite_declarations(css, cx, None)),
                Ok(Rewrite::Unchanged),
                "{css:?}"
            );
        }
    }

    #[test]
    fn urls_in_declarations() {
        assert_eq!(decl("fill:url(#a)"), "fill:url(#sm0-a)");
        assert_eq!(decl("fill:URL( '#a' )"), "fill:url(#sm0-a)");
        assert_eq!(decl("background:url(http://x/y.png)"), "background:none");
        assert_eq!(
            decl("background:url(\"//x/y.png\") red"),
            "background:none red"
        );
        assert_eq!(
            decl("background:url(data:image/png;base64,AAA)"),
            "background:url(data:image/png;base64,AAA)"
        );
        assert_eq!(decl("cursor:url(javascript:x),auto"), "cursor:none,auto");
        // CSS Syntax: `(` in an unquoted url makes a bad-url token that ends
        // at the first `)`; the rest stays as the tokenizer would see it.
        assert_eq!(
            decl("cursor:url(javascript:alert(1)),auto"),
            "cursor:none),auto"
        );
        assert_eq!(
            decl("x:'url(http://x)'"),
            "x:'url(http://x)'",
            "strings are not urls"
        );
        assert_eq!(decl("x:myurl(http://x)"), "x:myurl(http://x)");
        assert_eq!(decl("--v:url(http://x)"), "--v:none");
        assert_eq!(decl("a:url(x)b"), "a:none b", "no name runs into it");
    }

    #[test]
    fn escapes_and_hidden_loads_drop_the_declaration() {
        assert_eq!(decl("fill:red;background:u\\72l(http://x)"), "fill:red;");
        assert_eq!(decl("background:image-set('a.png' 1x)"), "");
        assert_eq!(decl("mask:-WEBKIT-image-set('a.png' 1x);x:y"), ";x:y");
        assert_eq!(decl("width:expression(alert(1));color:red"), ";color:red");
        assert_eq!(decl("b\\61ckground:red;c:d"), ";c:d");
        assert_eq!(decl("a:url( x y);b:c"), "a:none;b:c");
        assert_eq!(
            decl("a:url(var(--x))"),
            "a:none)",
            "a bad url ends at the first )"
        );
        assert_eq!(decl("a:x;@import 'y';b:c"), "a:x;;b:c");
        assert_eq!(
            decl("a:x;b{c:url(x)}d:e;f:g"),
            "a:x;;f:g",
            "no nested rules"
        );
        assert_eq!(decl("a:x}@import 'y';b:c"), "a:x};b:c");
        assert_eq!(decl("a:'b"), "", "a string left open");
        assert_eq!(decl("a:b/* c"), "", "a comment left open");
    }

    /// Review of #12, finding 1: CR (and CR LF, FF) is a newline to CSS, and
    /// a newline ends a string as a bad string.
    #[test]
    fn newlines_end_strings_as_in_css() {
        for nl in ["\r", "\r\n", "\x0c", "\n"] {
            let css = format!("x:'{nl};background:url(https://evil/a.png);y:'");
            assert_eq!(decl(&css), ";background:none;", "{nl:?}");
        }
        assert_eq!(decl("a:b\rc"), "a:b\nc", "preprocessed output");
    }

    /// Review of #12, finding 2: an unquoted `url(` is one token, so a `(`
    /// or `/*` inside it neither nests nor comments out the rest.
    #[test]
    fn unquoted_urls_are_one_token() {
        assert_eq!(
            decl("a:url(a(b);b:image-set(\"https://evil/g2.png\" 1x)"),
            "a:none;"
        );
        assert_eq!(
            decl("a:url(/*);b:url(https://evil/g4.png)/**/"),
            "a:none;b:none/**/"
        );
    }

    #[test]
    fn allowed_properties() {
        let root: &[&str] = &["fill", "font-family"];
        assert_eq!(
            decl_only(
                "position:fixed;fill:url(#a);top:0;FILL:red;--x:1",
                Some(root)
            ),
            ";fill:url(#sm0-a);;FILL:red;"
        );
        assert_eq!(decl_only("fill:red", Some(root)), "fill:red");
    }

    #[test]
    fn presentation_values() {
        assert_eq!(value("url(#a)"), "url(#sm0-a)");
        assert_eq!(value("url('https://x/#a')"), "none");
        assert_eq!(value("u\\72l(https://x/#a)"), "<dropped>");
        assert_eq!(value("image-set('https://x' 1x)"), "<dropped>");
        assert_eq!(value("-webkit-image-set('https://x' 1x)"), "<dropped>");
        assert_eq!(value("image-set('https://x' 1x), auto"), "<dropped>");
        assert_eq!(value("url(x) red"), "none red");
        assert_eq!(value("red;@import 'x'"), "<dropped>");
        assert_eq!(value("#fff"), "#fff");
    }

    #[test]
    fn final_check_is_structure_free() {
        for (css, ok) in [
            ("a:url(#x)", true),
            ("a:url(data:image/png,x)", true),
            ("a:url('#x')", true),
            ("a{b:url(#x)}", true),
            ("a:url(x)", false),
            ("a:url( 'x')", false),
            ("a:url(var(--x))", false),
            ("a:src('x')", false),
            ("a:IMAGE('x')", false),
            ("@media x{}", false),
            ("a\\62 c", false),
            ("'a\nb'", false),
            ("'a", false),
            ("/* a", false),
        ] {
            assert_eq!(is_safe(css), ok, "{css:?}");
        }
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let css = format!("a:{}", "(".repeat(10_000));
        assert!(with_cx(|cx| rewrite_declarations(&css, cx, None)).is_err());
        // A property value is only scanned token by token.
        assert_eq!(
            with_cx(|cx| rewrite_value(&css[2..], cx)),
            Rewrite::Unchanged
        );
    }

    #[test]
    fn hostile_css_never_panics() {
        for css in [
            "url(",
            "url('",
            "url(\"#",
            "/*",
            "'",
            "\\",
            "a{",
            "}",
            "@import",
            "@media{",
            "font:",
            "font-family:",
            "x:url(#a",
            "u\\",
            "{{{}}}}}",
            "@a{@b{@c{",
            "a:{",
            "--x:{}{}",
            ";;}}{{",
        ] {
            let _ = with_cx(|cx| rewrite_declarations(css, cx, None));
            let _ = with_cx(|cx| rewrite_value(css, cx));
        }
    }

    #[test]
    fn css_string_escaping() {
        assert_eq!(css_string("Noto Sans", '\''), "'Noto Sans'");
        assert_eq!(css_string("it's", '\''), "'it\\'s'");
        assert_eq!(css_string("a\\b", '"'), "\"a\\\\b\"");
        assert_eq!(css_string("a\nb", '"'), "\"a\\a b\"");
    }

    #[test]
    fn inline_images() {
        assert!(is_inline_image("data:image/png;base64,AAAA"));
        assert!(is_inline_image(" DATA:Image/svg+xml,<svg/>"));
        assert!(!is_inline_image("data:text/html,<script>"));
        assert!(!is_inline_image("data:"));
    }
}
