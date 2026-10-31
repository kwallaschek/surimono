//! Tokenizer for WebIDL, following the lexical grammar in
//! <https://webidl.spec.whatwg.org/#idl-grammar> (`identifier`, `integer`,
//! `decimal`, `string`, `whitespace`, `comment`, `other`).

use crate::ParseError;

/// One token with the 1-based line and column where it starts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Token {
    pub kind: TokenKind,
    pub line: u32,
    pub column: u32,
}

/// Token kinds. Keywords are plain identifiers; the parser tells them apart.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TokenKind {
    /// `[_-]?[A-Za-z][0-9A-Z_a-z-]*`, kept as written (leading `_` included).
    Ident(String),
    /// An integer literal, as written.
    Integer(String),
    /// A decimal literal, as written.
    Decimal(String),
    /// A string literal without its quotes (WebIDL strings have no escapes).
    Str(String),
    /// `...`
    Ellipsis,
    /// Any other single character, such as `{`, `;` or `<`.
    Other(char),
}

/// Splits `src` into tokens, dropping whitespace and comments.
pub(crate) fn tokenize(src: &str) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;

    // Advances over chars[i..j], keeping line and column right.
    // Returns `j`, having moved line and column over chars[i..j].
    let advance = |i: usize, j: usize, line: &mut u32, col: &mut u32| {
        for &c in &chars[i..j] {
            if c == '\n' {
                *line += 1;
                *col = 1;
            } else {
                *col += 1;
            }
        }
        j
    };

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let (start_line, start_col) = (line, col);

        if c.is_whitespace() {
            i = advance(i, i + 1, &mut line, &mut col);
            continue;
        }
        if c == '/' && next == Some('/') {
            let mut j = i;
            while j < chars.len() && chars[j] != '\n' {
                j += 1;
            }
            i = advance(i, j, &mut line, &mut col);
            continue;
        }
        if c == '/' && next == Some('*') {
            let mut j = i + 2;
            loop {
                if j + 1 >= chars.len() {
                    return Err(ParseError::new(
                        start_line,
                        start_col,
                        "unterminated comment",
                    ));
                }
                if chars[j] == '*' && chars[j + 1] == '/' {
                    break;
                }
                j += 1;
            }
            i = advance(i, j + 2, &mut line, &mut col);
            continue;
        }

        let (kind, len) = if c == '"' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '"' {
                j += 1;
            }
            if j >= chars.len() {
                return Err(ParseError::new(
                    start_line,
                    start_col,
                    "unterminated string",
                ));
            }
            let s: String = chars[i + 1..j].iter().collect();
            (TokenKind::Str(s), j + 1 - i)
        } else if let Some(n) = match_number(&chars[i..]) {
            let text: String = chars[i..i + n.len].iter().collect();
            let kind = if n.decimal {
                TokenKind::Decimal(text)
            } else {
                TokenKind::Integer(text)
            };
            (kind, n.len)
        } else if let Some(n) = match_identifier(&chars[i..]) {
            (TokenKind::Ident(chars[i..i + n].iter().collect()), n)
        } else if c == '.' && next == Some('.') && chars.get(i + 2) == Some(&'.') {
            (TokenKind::Ellipsis, 3)
        } else {
            (TokenKind::Other(c), 1)
        };
        out.push(Token {
            kind,
            line: start_line,
            column: start_col,
        });
        i = advance(i, i + len, &mut line, &mut col);
    }
    Ok(out)
}

/// `[_-]?[A-Za-z][0-9A-Z_a-z-]*`; returns the match length.
fn match_identifier(s: &[char]) -> Option<usize> {
    let mut j = 0;
    if matches!(s.first(), Some('_' | '-')) {
        j = 1;
    }
    if !s.get(j).is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    j += 1;
    while s
        .get(j)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
    {
        j += 1;
    }
    Some(j)
}

struct NumberMatch {
    len: usize,
    decimal: bool,
}

/// Longest match of `decimal` or `integer`.
fn match_number(s: &[char]) -> Option<NumberMatch> {
    let dec = match_decimal(s);
    let int = match_integer(s);
    match (dec, int) {
        (Some(d), Some(n)) if d > n => Some(NumberMatch {
            len: d,
            decimal: true,
        }),
        (_, Some(n)) => Some(NumberMatch {
            len: n,
            decimal: false,
        }),
        (Some(d), None) => Some(NumberMatch {
            len: d,
            decimal: true,
        }),
        (None, None) => None,
    }
}

fn digits(s: &[char], from: usize, pred: impl Fn(char) -> bool) -> usize {
    let mut j = from;
    while s.get(j).is_some_and(|c| pred(*c)) {
        j += 1;
    }
    j - from
}

/// `-?([1-9][0-9]*|0[Xx][0-9A-Fa-f]+|0[0-7]*)`
fn match_integer(s: &[char]) -> Option<usize> {
    let sign = usize::from(s.first() == Some(&'-'));
    match s.get(sign) {
        Some('1'..='9') => Some(sign + 1 + digits(s, sign + 1, |c| c.is_ascii_digit())),
        Some('0') => {
            if matches!(s.get(sign + 1), Some('x' | 'X')) {
                let n = digits(s, sign + 2, |c| c.is_ascii_hexdigit());
                if n > 0 {
                    return Some(sign + 2 + n);
                }
            }
            Some(sign + 1 + digits(s, sign + 1, |c| ('0'..='7').contains(&c)))
        }
        _ => None,
    }
}

/// `-?(([0-9]+\.[0-9]*|[0-9]*\.[0-9]+)([Ee][+-]?[0-9]+)?|[0-9]+[Ee][+-]?[0-9]+)`
fn match_decimal(s: &[char]) -> Option<usize> {
    let sign = usize::from(s.first() == Some(&'-'));
    let int = digits(s, sign, |c| c.is_ascii_digit());
    let mut j = sign + int;
    let has_dot = s.get(j) == Some(&'.');
    if has_dot {
        let frac = digits(s, j + 1, |c| c.is_ascii_digit());
        if int == 0 && frac == 0 {
            return None;
        }
        j += 1 + frac;
    } else if int == 0 {
        return None;
    }
    // Exponent.
    if matches!(s.get(j), Some('e' | 'E')) {
        let mut k = j + 1;
        if matches!(s.get(k), Some('+' | '-')) {
            k += 1;
        }
        let e = digits(s, k, |c| c.is_ascii_digit());
        if e > 0 {
            return Some(k + e);
        }
    }
    has_dot.then_some(j)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str) -> Vec<TokenKind> {
        tokenize(s).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn numbers() {
        use TokenKind::*;
        assert_eq!(
            kinds("0 017 0x1F -5 1.5 .5 1e3 -2.5E-2 1."),
            vec![
                Integer("0".into()),
                Integer("017".into()),
                Integer("0x1F".into()),
                Integer("-5".into()),
                Decimal("1.5".into()),
                Decimal(".5".into()),
                Decimal("1e3".into()),
                Decimal("-2.5E-2".into()),
                Decimal("1.".into()),
            ]
        );
    }

    #[test]
    fn identifiers_strings_comments() {
        use TokenKind::*;
        assert_eq!(
            kinds("_interface -Infinity /* x\n y */ \"a b\" // c\n..."),
            vec![
                Ident("_interface".into()),
                Ident("-Infinity".into()),
                Str("a b".into()),
                Ellipsis,
            ]
        );
    }

    #[test]
    fn positions() {
        let t = tokenize("a\n  b").unwrap();
        assert_eq!((t[1].line, t[1].column), (2, 3));
    }

    #[test]
    fn unterminated() {
        assert!(tokenize("/* x").is_err());
        assert!(tokenize("\"x").is_err());
    }
}
