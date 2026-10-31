//! A CSS Syntax Level 3 tokenizer.
//!
//! Implements "Tokenization" (§4) of <https://www.w3.org/TR/css-syntax-3/>
//! over input that went through [`preprocess`] (§3.3), the way a browser
//! tokenizes a style sheet or a `style` attribute. Every rewrite decision in
//! the parent module is made on these tokens, so the sanitiser and the browser
//! agree on where strings, comments, `url(...)` and blocks begin and end.
//!
//! Comments are kept as [`Kind::Comment`] tokens (the spec consumes them
//! silently) so that the tokens tile the input exactly: token `n` ends where
//! token `n + 1` starts.

use std::borrow::Cow;
use std::ops::Range;

/// CSS Syntax 3 §3.3: CR LF, CR and FF become LF, and NUL becomes U+FFFD.
/// Borrowed when there is nothing to change.
pub(crate) fn preprocess(css: &str) -> Cow<'_, str> {
    if !css.bytes().any(|b| matches!(b, b'\r' | b'\x0c' | 0)) {
        return Cow::Borrowed(css);
    }
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\x0c' => out.push('\n'),
            '\0' => out.push(char::REPLACEMENT_CHARACTER),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// A token's type (§4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Ident,
    /// A function token: the name and the `(`.
    Function,
    AtKeyword,
    Hash,
    String,
    BadString,
    Url,
    BadUrl,
    Delim(char),
    Number,
    Percentage,
    Dimension,
    Whitespace,
    /// A comment (not a token in the spec; kept so tokens tile the input).
    Comment,
    Cdo,
    Cdc,
    Colon,
    Semicolon,
    Comma,
    OpenSquare,
    CloseSquare,
    OpenParen,
    CloseParen,
    OpenCurly,
    CloseCurly,
}

/// One token of the input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) kind: Kind,
    /// Where it is in the (preprocessed) input.
    pub(crate) range: Range<usize>,
    /// The decoded name of an ident, function (without `(`), at-keyword
    /// (without `@`) or hash (without `#`); the decoded value of a string or
    /// url; the unit of a dimension. Empty for other kinds.
    pub(crate) value: String,
    /// An escape was decoded in a name, unit or url (outside a string).
    pub(crate) escaped: bool,
    /// A string, url or comment that the end of the input closed.
    pub(crate) unclosed: bool,
}

/// Splits preprocessed CSS into tokens that tile it.
pub(crate) fn tokenize(css: &str) -> Vec<Token> {
    let mut lexer = Lexer {
        s: css,
        b: css.as_bytes(),
        i: 0,
    };
    let mut out = Vec::new();
    while let Some(t) = lexer.next_token() {
        out.push(t);
    }
    out
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

fn is_ident_byte(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit() || b == b'-'
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n')
}

fn is_non_printable(b: u8) -> bool {
    matches!(b, 0x00..=0x08 | 0x0b | 0x0e..=0x1f | 0x7f)
}

struct Lexer<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl Lexer<'_> {
    fn at(&self, p: usize) -> Option<u8> {
        self.b.get(p).copied()
    }

    /// Length in bytes of the character starting at `p` (at least 1).
    fn char_len(&self, p: usize) -> usize {
        self.s[p..].chars().next().map_or(1, char::len_utf8)
    }

    /// §4.3.8: `\` not followed by a newline (the end of input counts).
    fn valid_escape(&self, p: usize) -> bool {
        self.at(p) == Some(b'\\') && self.at(p + 1) != Some(b'\n')
    }

    /// §4.3.9: would the code points at `p` start an ident sequence?
    fn starts_ident(&self, p: usize) -> bool {
        match self.at(p) {
            Some(b'-') => match self.at(p + 1) {
                Some(c) if is_ident_start(c) || c == b'-' => true,
                _ => self.valid_escape(p + 1),
            },
            Some(c) if is_ident_start(c) => true,
            Some(b'\\') => self.valid_escape(p),
            _ => false,
        }
    }

    /// §4.3.10: would the code points at `p` start a number?
    fn starts_number(&self, p: usize) -> bool {
        let digit = |q: usize| self.at(q).is_some_and(|c| c.is_ascii_digit());
        match self.at(p) {
            Some(b'+' | b'-') => digit(p + 1) || (self.at(p + 1) == Some(b'.') && digit(p + 2)),
            Some(b'.') => digit(p + 1),
            Some(c) => c.is_ascii_digit(),
            None => false,
        }
    }

    /// §4.3.7: consumes an escape at `self.i` (a valid escape) into `out`.
    fn escape(&mut self, out: &mut String) {
        self.i += 1; // the backslash
        let Some(c) = self.at(self.i) else {
            out.push(char::REPLACEMENT_CHARACTER);
            return;
        };
        if c.is_ascii_hexdigit() {
            let start = self.i;
            while self.i - start < 6 && self.at(self.i).is_some_and(|c| c.is_ascii_hexdigit()) {
                self.i += 1;
            }
            let cp = u32::from_str_radix(&self.s[start..self.i], 16).unwrap_or(0);
            if self.at(self.i).is_some_and(is_ws) {
                self.i += 1;
            }
            let ch = match cp {
                0 | 0xd800..=0xdfff => None,
                cp => char::from_u32(cp),
            };
            out.push(ch.unwrap_or(char::REPLACEMENT_CHARACTER));
        } else {
            let len = self.char_len(self.i);
            out.push_str(&self.s[self.i..self.i + len]);
            self.i += len;
        }
    }

    /// §4.3.11: consumes an ident sequence into `out`; true if it decoded an
    /// escape.
    fn ident_sequence(&mut self, out: &mut String) -> bool {
        let mut escaped = false;
        loop {
            let start = self.i;
            while self.at(self.i).is_some_and(is_ident_byte) {
                self.i += 1;
            }
            // Runs end at an ASCII byte or the end, so they are whole chars.
            out.push_str(&self.s[start..self.i]);
            if self.valid_escape(self.i) {
                self.escape(out);
                escaped = true;
            } else {
                return escaped;
            }
        }
    }

    fn token(&self, kind: Kind, start: usize) -> Token {
        Token {
            kind,
            range: start..self.i,
            value: String::new(),
            escaped: false,
            unclosed: false,
        }
    }

    /// §4.3.1: consumes one token (a comment counts as one).
    fn next_token(&mut self) -> Option<Token> {
        let start = self.i;
        let c = self.at(start)?;
        if c == b'/' && self.at(start + 1) == Some(b'*') {
            let mut t = match self.s[start + 2..].find("*/") {
                Some(k) => {
                    self.i = start + 2 + k + 2;
                    self.token(Kind::Comment, start)
                }
                None => {
                    self.i = self.b.len();
                    let mut t = self.token(Kind::Comment, start);
                    t.unclosed = true;
                    t
                }
            };
            t.range = start..self.i;
            return Some(t);
        }
        let single = |lexer: &mut Self, kind: Kind| {
            lexer.i += 1;
            lexer.token(kind, start)
        };
        let t = match c {
            c if is_ws(c) => {
                while self.at(self.i).is_some_and(is_ws) {
                    self.i += 1;
                }
                self.token(Kind::Whitespace, start)
            }
            b'"' | b'\'' => self.string(c),
            b'#' => {
                if self.at(start + 1).is_some_and(is_ident_byte) || self.valid_escape(start + 1) {
                    self.i += 1;
                    let mut value = String::new();
                    let escaped = self.ident_sequence(&mut value);
                    let mut t = self.token(Kind::Hash, start);
                    t.value = value;
                    t.escaped = escaped;
                    t
                } else {
                    single(self, Kind::Delim('#'))
                }
            }
            b'(' => single(self, Kind::OpenParen),
            b')' => single(self, Kind::CloseParen),
            b'+' | b'.' if self.starts_number(start) => self.numeric(),
            b',' => single(self, Kind::Comma),
            b'-' => {
                if self.starts_number(start) {
                    self.numeric()
                } else if self.b[start + 1..].starts_with(b"->") {
                    self.i += 3;
                    self.token(Kind::Cdc, start)
                } else if self.starts_ident(start) {
                    self.ident_like()
                } else {
                    single(self, Kind::Delim('-'))
                }
            }
            b':' => single(self, Kind::Colon),
            b';' => single(self, Kind::Semicolon),
            b'<' if self.b[start + 1..].starts_with(b"!--") => {
                self.i += 4;
                self.token(Kind::Cdo, start)
            }
            b'@' if self.starts_ident(start + 1) => {
                self.i += 1;
                let mut value = String::new();
                let escaped = self.ident_sequence(&mut value);
                let mut t = self.token(Kind::AtKeyword, start);
                t.value = value;
                t.escaped = escaped;
                t
            }
            b'[' => single(self, Kind::OpenSquare),
            b']' => single(self, Kind::CloseSquare),
            b'{' => single(self, Kind::OpenCurly),
            b'}' => single(self, Kind::CloseCurly),
            b'\\' if self.valid_escape(start) => self.ident_like(),
            c if c.is_ascii_digit() => self.numeric(),
            c if is_ident_start(c) => self.ident_like(),
            _ => {
                let len = self.char_len(start);
                let ch = self.s[start..].chars().next().unwrap_or('\0');
                self.i += len;
                self.token(Kind::Delim(ch), start)
            }
        };
        Some(t)
    }

    /// §4.3.5: a string token ending at `quote` (at `self.i`).
    fn string(&mut self, quote: u8) -> Token {
        let start = self.i;
        self.i += 1;
        let mut value = String::new();
        loop {
            match self.at(self.i) {
                None => {
                    let mut t = self.token(Kind::String, start);
                    t.value = value;
                    t.unclosed = true;
                    return t;
                }
                Some(c) if c == quote => {
                    self.i += 1;
                    let mut t = self.token(Kind::String, start);
                    t.value = value;
                    return t;
                }
                // The newline is not part of the bad string.
                Some(b'\n') => return self.token(Kind::BadString, start),
                Some(b'\\') => match self.at(self.i + 1) {
                    None => self.i += 1,
                    Some(b'\n') => self.i += 2,
                    Some(_) => self.escape(&mut value),
                },
                Some(_) => {
                    let len = self.char_len(self.i);
                    value.push_str(&self.s[self.i..self.i + len]);
                    self.i += len;
                }
            }
        }
    }

    /// §4.3.3: a number, percentage or dimension.
    fn numeric(&mut self) -> Token {
        let start = self.i;
        let digits = |lexer: &mut Self| {
            while lexer.at(lexer.i).is_some_and(|c| c.is_ascii_digit()) {
                lexer.i += 1;
            }
        };
        if matches!(self.at(self.i), Some(b'+' | b'-')) {
            self.i += 1;
        }
        digits(self);
        if self.at(self.i) == Some(b'.') && self.at(self.i + 1).is_some_and(|c| c.is_ascii_digit())
        {
            self.i += 1;
            digits(self);
        }
        if matches!(self.at(self.i), Some(b'e' | b'E')) {
            let sign = usize::from(matches!(self.at(self.i + 1), Some(b'+' | b'-')));
            if self
                .at(self.i + 1 + sign)
                .is_some_and(|c| c.is_ascii_digit())
            {
                self.i += 1 + sign;
                digits(self);
            }
        }
        if self.starts_ident(self.i) {
            let mut unit = String::new();
            let escaped = self.ident_sequence(&mut unit);
            let mut t = self.token(Kind::Dimension, start);
            t.value = unit;
            t.escaped = escaped;
            t
        } else if self.at(self.i) == Some(b'%') {
            self.i += 1;
            self.token(Kind::Percentage, start)
        } else {
            self.token(Kind::Number, start)
        }
    }

    /// §4.3.4: an ident, function, url or bad-url token.
    fn ident_like(&mut self) -> Token {
        let start = self.i;
        let mut name = String::new();
        let escaped = self.ident_sequence(&mut name);
        let kind = if self.at(self.i) == Some(b'(') {
            self.i += 1;
            if name.eq_ignore_ascii_case("url") {
                while self.at(self.i).is_some_and(is_ws) && self.at(self.i + 1).is_some_and(is_ws) {
                    self.i += 1;
                }
                let quote_next = |c: Option<u8>| matches!(c, Some(b'"' | b'\''));
                let next = self.at(self.i);
                if quote_next(next) || (next.is_some_and(is_ws) && quote_next(self.at(self.i + 1)))
                {
                    Kind::Function
                } else {
                    let mut t = self.url(start);
                    t.escaped |= escaped;
                    return t;
                }
            } else {
                Kind::Function
            }
        } else {
            Kind::Ident
        };
        let mut t = self.token(kind, start);
        t.value = name;
        t.escaped = escaped;
        t
    }

    /// §4.3.6: a url token; `self.i` is just after `url(`.
    fn url(&mut self, start: usize) -> Token {
        let mut value = String::new();
        let mut escaped = false;
        while self.at(self.i).is_some_and(is_ws) {
            self.i += 1;
        }
        loop {
            match self.at(self.i) {
                None => {
                    let mut t = self.token(Kind::Url, start);
                    t.value = value;
                    t.escaped = escaped;
                    t.unclosed = true;
                    return t;
                }
                Some(b')') => {
                    self.i += 1;
                    let mut t = self.token(Kind::Url, start);
                    t.value = value;
                    t.escaped = escaped;
                    return t;
                }
                Some(c) if is_ws(c) => {
                    while self.at(self.i).is_some_and(is_ws) {
                        self.i += 1;
                    }
                    match self.at(self.i) {
                        None | Some(b')') => {}
                        Some(_) => return self.bad_url(start),
                    }
                }
                Some(b'"' | b'\'' | b'(') => return self.bad_url(start),
                Some(c) if is_non_printable(c) => return self.bad_url(start),
                Some(b'\\') => {
                    if self.valid_escape(self.i) {
                        self.escape(&mut value);
                        escaped = true;
                    } else {
                        return self.bad_url(start);
                    }
                }
                Some(_) => {
                    let len = self.char_len(self.i);
                    value.push_str(&self.s[self.i..self.i + len]);
                    self.i += len;
                }
            }
        }
    }

    /// §4.3.14: the rest of a bad url, through the first unescaped `)`.
    fn bad_url(&mut self, start: usize) -> Token {
        let mut sink = String::new();
        let mut closed = false;
        while let Some(c) = self.at(self.i) {
            if c == b')' {
                self.i += 1;
                closed = true;
                break;
            }
            if self.valid_escape(self.i) {
                self.escape(&mut sink);
            } else {
                self.i += self.char_len(self.i);
            }
        }
        let mut t = self.token(Kind::BadUrl, start);
        t.unclosed = !closed;
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(css: &str) -> Vec<(Kind, &str)> {
        tokenize(css)
            .into_iter()
            .map(|t| (t.kind, &css[t.range]))
            .collect()
    }

    fn values(css: &str) -> Vec<(Kind, String)> {
        tokenize(css)
            .into_iter()
            .filter(|t| !matches!(t.kind, Kind::Whitespace | Kind::Comment))
            .map(|t| (t.kind, t.value))
            .collect()
    }

    #[test]
    fn preprocessing() {
        assert!(matches!(preprocess("a\nb"), Cow::Borrowed(_)));
        assert_eq!(preprocess("a\r\nb\rc\x0cd\0"), "a\nb\nc\nd\u{fffd}");
    }

    #[test]
    fn tokens_tile_the_input() {
        for css in [
            "a{b:c}",
            "/* x */ @media (x) { .a { fill: url( x ) } }",
            "url(a(b) 'x\n y' \\",
            "#\\31 x .5e+3px 10% -->-x <!--",
            "日本{語:\"x\\\ny\"}",
        ] {
            let t = tokenize(css);
            let mut pos = 0;
            for tok in &t {
                assert_eq!(tok.range.start, pos, "{css:?}");
                pos = tok.range.end;
            }
            assert_eq!(pos, css.len(), "{css:?}");
        }
    }

    /// Unquoted `url(` is one token through the first `)`; `(`, quotes or
    /// inner whitespace make it a bad url (§4.3.6, §4.3.14).
    #[test]
    fn url_tokens() {
        assert_eq!(values("url( a.png )"), [(Kind::Url, "a.png".into())]);
        assert_eq!(kinds("url(a(b);x")[0], (Kind::BadUrl, "url(a(b)"));
        assert_eq!(kinds("url(/*);x")[0], (Kind::Url, "url(/*)"));
        assert_eq!(values("url(/*)")[0].1, "/*");
        assert_eq!(kinds("url(a b)")[0], (Kind::BadUrl, "url(a b)"));
        assert_eq!(kinds("url(a\\)b)")[0], (Kind::Url, "url(a\\)b)"));
        assert_eq!(kinds("url(a'b)c)")[0], (Kind::BadUrl, "url(a'b)"));
        assert_eq!(
            values("u\\72l(https://x)"),
            [(Kind::Url, "https://x".into())]
        );
        assert!(tokenize("u\\72l(x)")[0].escaped);
        assert_eq!(
            kinds("url( 'x')"),
            [
                (Kind::Function, "url("),
                (Kind::Whitespace, " "),
                (Kind::String, "'x'"),
                (Kind::CloseParen, ")")
            ]
        );
        assert_eq!(kinds("URL(  \"x\")")[0], (Kind::Function, "URL( "));
        assert!(tokenize("url(x")[0].unclosed);
    }

    #[test]
    fn strings_end_at_newlines() {
        assert_eq!(
            kinds("'a\nb'"),
            [
                (Kind::BadString, "'a"),
                (Kind::Whitespace, "\n"),
                (Kind::Ident, "b"),
                (Kind::String, "'")
            ]
        );
        assert_eq!(values("'a\\\nb'"), [(Kind::String, "ab".into())]);
        assert_eq!(values("\"\\22\\41 x\""), [(Kind::String, "\"Ax".into())]);
        assert!(tokenize("'abc")[0].unclosed);
    }

    #[test]
    fn names_and_escapes() {
        assert_eq!(
            values("@\\69mport x"),
            [
                (Kind::AtKeyword, "import".into()),
                (Kind::Ident, "x".into())
            ]
        );
        assert_eq!(
            values("-webkit-image-set( #a1 #1a"),
            [
                (Kind::Function, "-webkit-image-set".into()),
                (Kind::Hash, "a1".into()),
                (Kind::Hash, "1a".into())
            ]
        );
        assert_eq!(
            values("\\0 \\110000 \\d800")[0].1,
            "\u{fffd}\u{fffd}\u{fffd}"
        );
        assert_eq!(kinds("\\\nx")[0], (Kind::Delim('\\'), "\\"));
        assert_eq!(kinds("--x")[0], (Kind::Ident, "--x"));
        assert_eq!(kinds("-->")[0], (Kind::Cdc, "-->"));
        assert_eq!(kinds("-1px")[0], (Kind::Dimension, "-1px"));
        assert_eq!(kinds("1e3")[0], (Kind::Number, "1e3"));
        assert_eq!(kinds("1em")[0], (Kind::Dimension, "1em"));
        assert_eq!(kinds(".5%")[0], (Kind::Percentage, ".5%"));
        assert_eq!(kinds("@ x")[0], (Kind::Delim('@'), "@"));
    }

    #[test]
    fn comments() {
        assert_eq!(
            kinds("a/**/b/* x"),
            [
                (Kind::Ident, "a"),
                (Kind::Comment, "/**/"),
                (Kind::Ident, "b"),
                (Kind::Comment, "/* x")
            ]
        );
        assert!(tokenize("/* x")[0].unclosed);
        assert_eq!(kinds("'/*'")[0], (Kind::String, "'/*'"));
    }

    #[test]
    fn hostile_input_never_panics() {
        for css in [
            "\\",
            "'\\",
            "url(\\",
            "url(",
            "u\\",
            "#\\",
            "@\\",
            "1e",
            "1e+",
            "-\\",
            "+.",
            "<!-",
            "url(\u{1}",
            "\u{80}\\\u{10ffff}",
            "\"\\\u{e9}",
        ] {
            let t = tokenize(css);
            assert_eq!(t.last().map_or(0, |t| t.range.end), css.len(), "{css:?}");
        }
    }
}
