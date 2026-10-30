//! Minimal CSS rewriting for SVG `style` attributes and `<style>` sheets.
//!
//! Not a full CSS parser: it tracks strings, comments, parentheses and blocks
//! well enough to find every `url(...)`, `@import`, `font`/`font-family`
//! declaration and `#id` selector, and edits only those. Anything that could
//! hide a resource load from this scanner (CSS escapes outside strings,
//! `image-set()`, `expression()`) drops the declaration or rule.

use std::collections::HashSet;
use std::ops::Range;

use crate::css_font::FontSpec;
use crate::fonts::FontSet;

/// Nesting deeper than this is rejected rather than scanned.
const MAX_DEPTH: usize = 64;

/// What the rewriter needs from the SVG around the CSS.
pub(crate) struct CssContext<'a> {
    pub(crate) fonts: &'a FontSet,
    /// Prefix for `#id` references.
    pub(crate) id_prefix: &'a str,
    /// Ids defined in the SVG; only these `#id` selectors are prefixed.
    pub(crate) ids: &'a HashSet<String>,
    /// Quote character for family names (the one the surrounding XML does
    /// not use as attribute delimiter).
    pub(crate) quote: char,
}

/// Rewrites a declaration list (a `style` attribute). `Ok(None)`: unchanged.
pub(crate) fn rewrite_declarations(
    css: &str,
    cx: &CssContext<'_>,
) -> Result<Option<String>, String> {
    let mut sc = Scanner::new(css, cx);
    let mut i = 0;
    while i < css.len() {
        // A stray `}` in a style attribute ends nothing; step over it.
        i = sc.declarations(i, 0)? + 1;
    }
    Ok(sc.finish())
}

/// Rewrites a style sheet (`<style>` text). `Ok(None)`: unchanged.
pub(crate) fn rewrite_stylesheet(css: &str, cx: &CssContext<'_>) -> Result<Option<String>, String> {
    let mut sc = Scanner::new(css, cx);
    let mut i = 0;
    while i < css.len() {
        i = sc.rules(i, 0)? + 1;
    }
    Ok(sc.finish())
}

/// True if every string and comment in `css` is closed. A style sheet that
/// ends inside one is not something a chart library writes, and whatever
/// follows the opening quote is text no rule can use: such a sheet is dropped.
pub(crate) fn strings_and_comments_closed(css: &str) -> bool {
    let b = css.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            q @ (b'"' | b'\'') => {
                i += 1;
                loop {
                    match b.get(i) {
                        None | Some(b'\n' | b'\r' | b'\x0c') => return false,
                        Some(b'\\') => i += 2,
                        Some(&c) if c == q => break,
                        Some(_) => i += 1,
                    }
                }
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => match css[i + 2..].find("*/") {
                Some(k) => i += 2 + k + 2,
                None => return false,
            },
            _ => i += 1,
        }
    }
    true
}

/// Rewrites only the `url(...)` tokens in a presentation attribute value
/// such as `fill="url(#g)"`. `Ok(None)`: unchanged.
pub(crate) fn rewrite_urls(value: &str, cx: &CssContext<'_>) -> Result<Option<String>, String> {
    let mut sc = Scanner::new(value, cx);
    if sc.has_escape_outside_strings(0..value.len()) {
        // An escaped `url(` would slip past the scanner: drop the value.
        return Ok(Some("none".to_string()));
    }
    sc.urls(0..value.len());
    Ok(sc.finish())
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

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b >= 0x80
}

struct Scanner<'a> {
    s: &'a str,
    b: &'a [u8],
    cx: &'a CssContext<'a>,
    edits: Vec<(Range<usize>, String)>,
}

impl<'a> Scanner<'a> {
    fn new(s: &'a str, cx: &'a CssContext<'a>) -> Self {
        Scanner {
            s,
            b: s.as_bytes(),
            cx,
            edits: Vec::new(),
        }
    }

    fn finish(mut self) -> Option<String> {
        if self.edits.is_empty() {
            return None;
        }
        self.edits.sort_by_key(|(r, _)| (r.start, r.end));
        let mut out = String::with_capacity(self.s.len() + 64);
        let mut pos = 0;
        for (r, new) in &self.edits {
            if r.start < pos {
                continue; // nested in an earlier edit (a dropped rule); already gone
            }
            out.push_str(&self.s[pos..r.start]);
            out.push_str(new);
            pos = r.end;
        }
        out.push_str(&self.s[pos..]);
        Some(out)
    }

    /// Index just past a string starting at `i` (the quote). Unterminated
    /// strings end at a newline or the end, as in CSS.
    fn skip_string(&self, i: usize) -> usize {
        let q = self.b[i];
        let mut j = i + 1;
        while j < self.b.len() {
            match self.b[j] {
                b'\\' => j += 2,
                b'\n' => return j,
                c if c == q => return j + 1,
                _ => j += 1,
            }
        }
        self.b.len()
    }

    /// Index just past a comment starting at `i` (`/*`).
    fn skip_comment(&self, i: usize) -> usize {
        match self.s[i + 2..].find("*/") {
            Some(k) => i + 2 + k + 2,
            None => self.b.len(),
        }
    }

    fn skip_ws_comments(&self, mut i: usize) -> usize {
        loop {
            while i < self.b.len() && self.b[i].is_ascii_whitespace() {
                i += 1;
            }
            let rest = self.b.get(i..).unwrap_or(&[]);
            if rest.is_empty() {
                return i.min(self.b.len());
            } else if rest.starts_with(b"/*") {
                i = self.skip_comment(i);
            } else if rest.starts_with(b"<!--") {
                i += 4; // CDO, ignored by CSS
            } else if rest.starts_with(b"-->") {
                i += 3; // CDC
            } else {
                return i;
            }
        }
    }

    /// Finds the first of `stops` at parenthesis depth 0, outside strings and
    /// comments. Returns its index (or the end) and the byte found.
    fn find_stop(&self, mut i: usize, stops: &[u8]) -> (usize, Option<u8>) {
        let mut parens = 0usize;
        while i < self.b.len() {
            let c = self.b[i];
            match c {
                b'"' | b'\'' => {
                    i = self.skip_string(i);
                    continue;
                }
                b'/' if self.b.get(i + 1) == Some(&b'*') => {
                    i = self.skip_comment(i);
                    continue;
                }
                b'\\' => {
                    i += 2;
                    continue;
                }
                b'(' => parens += 1,
                b')' => parens = parens.saturating_sub(1),
                _ if parens == 0 && stops.contains(&c) => return (i, Some(c)),
                _ => {}
            }
            i += 1;
        }
        (self.b.len(), None)
    }

    fn has_escape_outside_strings(&self, r: Range<usize>) -> bool {
        let mut i = r.start;
        while i < r.end {
            match self.b[i] {
                b'"' | b'\'' => i = self.skip_string(i),
                b'/' if self.b.get(i + 1) == Some(&b'*') => i = self.skip_comment(i),
                b'\\' => return true,
                _ => i += 1,
            }
        }
        false
    }

    /// Scans a block of declarations from `i` to the `}` that closes it (its
    /// index is returned) or the end.
    fn declarations(&mut self, mut i: usize, depth: usize) -> Result<usize, String> {
        if depth > MAX_DEPTH {
            return Err("CSS nests too deeply".into());
        }
        loop {
            i = self.skip_ws_comments(i);
            match self.b.get(i) {
                None => return Ok(self.b.len()),
                Some(b'}') => return Ok(i),
                Some(b';') => {
                    i += 1;
                    continue;
                }
                _ => {}
            }
            let start = i;
            let (end, stop) = self.find_stop(i, b";{}");
            if stop == Some(b'{') {
                // A nested rule (CSS nesting): selector, then a block.
                self.selector(start..end);
                let close = self.declarations(end + 1, depth + 1)?;
                i = close + 1;
                continue;
            }
            self.declaration(start..end);
            i = end;
            if stop == Some(b'}') {
                return Ok(end);
            }
        }
    }

    fn declaration(&mut self, r: Range<usize>) {
        // Anything that could load a resource without a plain `url(` token
        // loses the whole declaration.
        let lower = self.s[r.clone()].to_ascii_lowercase();
        if self.has_escape_outside_strings(r.clone())
            || lower.contains("image-set(")
            || lower.contains("image(")
            || lower.contains("src(")
            || lower.contains("expression(")
            || lower.contains("element(")
        {
            self.edits.push((r, String::new()));
            return;
        }
        let Some(colon) = self.s[r.clone()].find(':').map(|k| r.start + k) else {
            self.urls(r);
            return;
        };
        let name = self.s[r.start..colon].trim().to_ascii_lowercase();
        let value = colon + 1..r.end;
        if name == "font-family" || name == "font" {
            let core = value_core(self.s, value.clone());
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
            return;
        }
        self.urls(value);
    }

    /// Prefixes `#id` selectors that name ids defined in this SVG, and
    /// ECharts' `.zr0-cls-1` class selectors.
    fn selector(&mut self, r: Range<usize>) {
        let mut i = r.start;
        while i < r.end {
            match self.b[i] {
                b'"' | b'\'' => {
                    i = self.skip_string(i);
                    continue;
                }
                b'/' if self.b.get(i + 1) == Some(&b'*') => {
                    i = self.skip_comment(i);
                    continue;
                }
                b'\\' => {
                    i += 2;
                    continue;
                }
                b'#' | b'.' => {
                    let start = i + 1;
                    let mut j = start;
                    while j < r.end && is_ident_byte(self.b[j]) {
                        j += 1;
                    }
                    let name = &self.s[start..j];
                    let prefixed = if self.b[i] == b'#' {
                        self.cx.ids.contains(name)
                    } else {
                        super::is_echarts_class(name)
                    };
                    if j > start && prefixed {
                        self.edits
                            .push((start..start, self.cx.id_prefix.to_string()));
                    }
                    i = j;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// Scans rules from `i` to the `}` closing the enclosing block (index
    /// returned) or the end.
    fn rules(&mut self, mut i: usize, depth: usize) -> Result<usize, String> {
        if depth > MAX_DEPTH {
            return Err("CSS nests too deeply".into());
        }
        loop {
            i = self.skip_ws_comments(i);
            match self.b.get(i) {
                None => return Ok(self.b.len()),
                Some(b'}') => return Ok(i),
                _ => {}
            }
            let start = i;
            let (end, stop) = self.find_stop(i, b";{}");
            let prelude = self.s[start..end].trim();
            let lower = prelude.to_ascii_lowercase();
            match stop {
                Some(b'{') => {
                    let at = lower.strip_prefix('@').map(|rest| {
                        rest.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                            .next()
                            .unwrap_or("")
                            .to_string()
                    });
                    let close = match at.as_deref() {
                        _ if prelude.starts_with('@') && prelude.contains('\\') => {
                            // An escaped at-keyword could be anything: drop the rule.
                            let close = self.skip_block(end + 1);
                            self.edits
                                .push((start..(close + 1).min(self.b.len()), String::new()));
                            close
                        }
                        Some(
                            "media" | "supports" | "layer" | "container" | "document"
                            | "-moz-document" | "scope" | "starting-style" | "keyframes"
                            | "-webkit-keyframes" | "-moz-keyframes",
                        ) => self.rules(end + 1, depth + 1)?,
                        Some("font-face") => {
                            // Fonts come from the registry only: `src: local()`
                            // would reach system fonts. Drop the rule.
                            let close = self.skip_block(end + 1);
                            self.edits
                                .push((start..(close + 1).min(self.b.len()), String::new()));
                            close
                        }
                        Some(_) => self.declarations(end + 1, depth + 1)?,
                        None => {
                            self.selector(start..end);
                            self.declarations(end + 1, depth + 1)?
                        }
                    };
                    i = close + 1;
                }
                Some(b'}') => {
                    // A prelude cut short by the block's end: drop it.
                    self.urls(start..end);
                    return Ok(end);
                }
                _ => {
                    // A statement at-rule (or garbage) up to `;`.
                    let stmt_end = if stop == Some(b';') { end + 1 } else { end };
                    if lower.starts_with("@import")
                        || (prelude.starts_with('@') && prelude.contains('\\'))
                    {
                        self.edits.push((start..stmt_end, String::new()));
                    } else {
                        self.urls(start..end);
                    }
                    i = stmt_end;
                }
            }
        }
    }

    /// Index of the `}` matching an opened block whose body starts at `i`.
    fn skip_block(&self, mut i: usize) -> usize {
        let mut depth = 1usize;
        loop {
            let (end, stop) = self.find_stop(i, b"{}");
            match stop {
                Some(b'{') => depth += 1,
                Some(_) => {
                    depth -= 1;
                    if depth == 0 {
                        return end;
                    }
                }
                None => return end,
            }
            i = end + 1;
        }
    }

    /// Rewrites every `url(...)` in `r`: `#id` gets the island prefix,
    /// `data:image/...` stays, anything else becomes `none`.
    fn urls(&mut self, r: Range<usize>) {
        let mut i = r.start;
        while i < r.end {
            match self.b[i] {
                b'"' | b'\'' => {
                    i = self.skip_string(i);
                    continue;
                }
                b'/' if self.b.get(i + 1) == Some(&b'*') => {
                    i = self.skip_comment(i);
                    continue;
                }
                b'u' | b'U'
                    if self.b.len() >= i + 4
                        && self.b[i..i + 4].eq_ignore_ascii_case(b"url(")
                        && (i == 0 || !is_ident_byte(self.b[i - 1])) =>
                {
                    let (end, url) = self.url_token(i + 4);
                    let replacement = classify_url(&url, self.cx.id_prefix);
                    if let Some(new) = replacement {
                        self.edits.push((i..end, new));
                    }
                    i = end;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// Parses a url token's argument starting at `i` (after `url(`). Returns
    /// the index after the closing `)` and the URL text.
    fn url_token(&self, i: usize) -> (usize, String) {
        let mut j = i;
        while j < self.b.len() && self.b[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut url = String::new();
        if j < self.b.len() && (self.b[j] == b'"' || self.b[j] == b'\'') {
            let end = self.skip_string(j);
            let inner_end = if end > j + 1 && self.b[end - 1] == self.b[j] {
                end - 1
            } else {
                end
            };
            url.push_str(&unescape(&self.s[j + 1..inner_end]));
            j = end;
        } else {
            let start = j;
            while j < self.b.len() && self.b[j] != b')' {
                j += 1;
            }
            url.push_str(self.s[start..j].trim());
        }
        // Up to and including the closing parenthesis.
        while j < self.b.len() && self.b[j] != b')' {
            j += 1;
        }
        ((j + 1).min(self.b.len()), url)
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

/// Resolves backslash escapes in a CSS string body (enough for URLs).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            hex.extend(chars.next());
        }
        if hex.is_empty() {
            match chars.next() {
                Some('\n') | None => {}
                Some(c) => out.push(c),
            }
        } else {
            if chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                chars.next();
            }
            let cp = u32::from_str_radix(&hex, 16).unwrap_or(0xfffd);
            out.push(char::from_u32(cp).unwrap_or(char::REPLACEMENT_CHARACTER));
        }
    }
    out
}

/// What to do with a URL inside `url()`: `None` keeps it as written.
fn classify_url(url: &str, prefix: &str) -> Option<String> {
    let trimmed = url.trim();
    if let Some(id) = trimmed.strip_prefix('#') {
        let new_id = format!("{prefix}{id}");
        if new_id
            .bytes()
            .all(|b| is_ident_byte(b) || b == b'.' || b == b':')
        {
            return Some(format!("url(#{new_id})"));
        }
        return Some(format!(
            "url(\"#{}\")",
            new_id.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    if is_inline_image(trimmed) {
        return None;
    }
    Some("none".to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn with_cx<R>(ids: &[&str], f: impl FnOnce(&CssContext<'_>) -> R) -> R {
        let fonts = FontSet::new();
        let ids: HashSet<String> = ids.iter().map(|s| s.to_string()).collect();
        let cx = CssContext {
            fonts: &fonts,
            id_prefix: "sm0-",
            ids: &ids,
            quote: '\'',
        };
        f(&cx)
    }

    fn decl(css: &str) -> String {
        with_cx(&["a"], |cx| rewrite_declarations(css, cx))
            .expect("ok")
            .unwrap_or_else(|| css.to_string())
    }

    fn sheet(css: &str) -> String {
        with_cx(&["a", "b"], |cx| rewrite_stylesheet(css, cx))
            .expect("ok")
            .unwrap_or_else(|| css.to_string())
    }

    #[test]
    fn untouched_css_is_unchanged() {
        for css in [
            "fill:#fff;stroke:red",
            "font-size:12px;",
            "",
            "  ",
            "a:b;;;c:d",
        ] {
            assert_eq!(
                with_cx(&[], |cx| rewrite_declarations(css, cx)),
                Ok(None),
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
    }

    #[test]
    fn escapes_and_hidden_loads_drop_the_declaration() {
        assert_eq!(decl("fill:red;background:u\\72l(http://x)"), "fill:red;");
        assert_eq!(decl("background:image-set('a.png' 1x)"), "");
        assert_eq!(decl("width:expression(alert(1));color:red"), ";color:red");
    }

    #[test]
    fn stylesheets() {
        assert_eq!(sheet("@import 'x.css';\n.a{fill:red}"), "\n.a{fill:red}");
        assert_eq!(sheet("@import url(x.css) screen;"), "");
        assert_eq!(sheet("@\\69mport 'x.css';.a{}"), ".a{}");
        assert_eq!(
            sheet("#a:hover{fill:url(#b)}"),
            "#sm0-a:hover{fill:url(#sm0-b)}"
        );
        assert_eq!(sheet("#zz{}"), "#zz{}", "unknown ids are left alone");
        assert_eq!(
            sheet("@media print{#a{fill:url(http://x)}}"),
            "@media print{#sm0-a{fill:none}}"
        );
        assert_eq!(
            sheet("@font-face{font-family:X;src:url(http://x/f.woff2)}.a{}"),
            ".a{}",
            "@font-face goes: fonts come from the registry"
        );
        assert_eq!(
            sheet("@media print{@FONT-FACE{font-family:Y;src:local(Arial)}}"),
            "@media print{}"
        );
        assert_eq!(
            sheet(".zr0-cls-1:hover{fill:red} .zr12-x .zrx-y .zr-1 #a.zr3-z{}"),
            ".sm0-zr0-cls-1:hover{fill:red} .sm0-zr12-x .zrx-y .zr-1 #sm0-a.sm0-zr3-z{}",
            "ECharts classes get the island prefix"
        );
        assert_eq!(
            sheet("@\\66ont-face{src:url(http://x)}.b{}"),
            ".b{}",
            "escaped at-keywords drop the rule"
        );
        assert_eq!(
            sheet(".a{fill:red}}.b{fill:url(x)}"),
            ".a{fill:red}}.b{fill:none}"
        );
    }

    #[test]
    fn open_strings_and_comments() {
        for ok in [
            "",
            "a{b:\"c\"}",
            "a{b:'c\\'d'}",
            "/* x */a{}",
            "a{b:\"/*\"}",
            "a\\\"b",
        ] {
            assert!(strings_and_comments_closed(ok), "{ok}");
        }
        for open in ["a{b:\"}", "a{b:'c\nd'}", "/* x", "a{b:\"c\\\"}", "x'"] {
            assert!(!strings_and_comments_closed(open), "{open}");
        }
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let css = "@media x{".repeat(10_000);
        assert!(with_cx(&[], |cx| rewrite_stylesheet(&css, cx)).is_err());
        let css = "a{".repeat(10_000);
        assert!(with_cx(&[], |cx| rewrite_declarations(&css, cx)).is_err());
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
        ] {
            let _ = with_cx(&["a"], |cx| rewrite_declarations(css, cx));
            let _ = with_cx(&["a"], |cx| rewrite_stylesheet(css, cx));
            let _ = with_cx(&["a"], |cx| rewrite_urls(css, cx));
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
