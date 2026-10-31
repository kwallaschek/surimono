//! Character references in attribute values, decoded the way an HTML
//! tokenizer decodes them (HTML Living Standard, 13.2.5.72 "Character
//! reference state" and the states after it, in an attribute value).
//!
//! lol_html hands attribute values over exactly as written. The pre-pass keeps
//! that raw text when it writes an attribute back out, but reads the meaning
//! of `data-renderer`, `data-width`, `data-height` and `id` from the decoded
//! value, as a browser would: `data-renderer="e&#99;harts"` names `echarts`.

use std::borrow::Cow;

use web_atoms::NAMED_ENTITIES;

/// `value` with its character references decoded.
pub(crate) fn decode_attribute(value: &str) -> Cow<'_, str> {
    if !value.contains('&') {
        return Cow::Borrowed(value);
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let consumed = if let Some(numeric) = after.strip_prefix('#') {
            numeric_reference(numeric, &mut out).map(|n| n + 1)
        } else {
            named_reference(after, &mut out)
        };
        match consumed {
            Some(n) => rest = &after[n..],
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Decodes `&#…` (after the `#`) into `out`; returns how many bytes of `s` it
/// used, or `None` when no digits follow (then the text stays as written).
fn numeric_reference(s: &str, out: &mut String) -> Option<usize> {
    let (radix, start) = match s.as_bytes().first() {
        Some(b'x' | b'X') => (16, 1),
        _ => (10, 0),
    };
    let digits = s[start..]
        .bytes()
        .take_while(|b| (*b as char).is_digit(radix))
        .count();
    if digits == 0 {
        return None;
    }
    let mut code: u32 = 0;
    for b in s[start..start + digits].bytes() {
        // Digits were checked above; anything past U+10FFFF saturates and
        // becomes U+FFFD below.
        let d = (b as char).to_digit(radix).unwrap_or(0);
        code = code.saturating_mul(radix).saturating_add(d);
    }
    let mut used = start + digits;
    if s[used..].starts_with(';') {
        used += 1;
    }
    out.push(numeric_char(code));
    Some(used)
}

/// The character a numeric reference stands for (13.2.5.80 "Numeric character
/// reference end state").
fn numeric_char(code: u32) -> char {
    match code {
        0 | 0xD800..=0xDFFF | 0x11_0000.. => '\u{FFFD}',
        0x80..=0x9F => windows_1252(code),
        _ => char::from_u32(code).unwrap_or('\u{FFFD}'),
    }
}

/// C1 control references map to what windows-1252 has at that byte.
fn windows_1252(code: u32) -> char {
    let mapped = match code {
        0x80 => 0x20AC,
        0x82 => 0x201A,
        0x83 => 0x0192,
        0x84 => 0x201E,
        0x85 => 0x2026,
        0x86 => 0x2020,
        0x87 => 0x2021,
        0x88 => 0x02C6,
        0x89 => 0x2030,
        0x8A => 0x0160,
        0x8B => 0x2039,
        0x8C => 0x0152,
        0x8E => 0x017D,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201C,
        0x94 => 0x201D,
        0x95 => 0x2022,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x98 => 0x02DC,
        0x99 => 0x2122,
        0x9A => 0x0161,
        0x9B => 0x203A,
        0x9C => 0x0153,
        0x9E => 0x017E,
        0x9F => 0x0178,
        other => other,
    };
    char::from_u32(mapped).unwrap_or('\u{FFFD}')
}

/// Decodes a named reference at the start of `s` (after the `&`) into `out`;
/// returns how many bytes it used, or `None` to leave the text as written.
///
/// The longest name in the table that `s` starts with wins. In an attribute, a
/// match without its `;` followed by `=` or an ASCII letter or digit is not a
/// reference (so `?a=1&copy=2` keeps `&copy`).
fn named_reference(s: &str, out: &mut String) -> Option<usize> {
    let run = s.bytes().take_while(u8::is_ascii_alphanumeric).count();
    if run == 0 {
        return None;
    }
    let with_semicolon = run + usize::from(s[run..].starts_with(';'));
    // Names are ASCII, so every length is a char boundary.
    let (len, (first, second)) =
        (1..=with_semicolon)
            .rev()
            .find_map(|n| match NAMED_ENTITIES.get(&s[..n]) {
                // The table also maps every prefix of a name to (0, 0).
                Some(&(0, _)) | None => None,
                Some(&cps) => Some((n, cps)),
            })?;
    let name = &s[..len];
    if !name.ends_with(';') {
        let next = s.as_bytes().get(len).copied();
        if next.is_some_and(|b| b == b'=' || b.is_ascii_alphanumeric()) {
            return None;
        }
    }
    out.push(char::from_u32(first).unwrap_or('\u{FFFD}'));
    if second != 0 {
        out.push(char::from_u32(second).unwrap_or('\u{FFFD}'));
    }
    Some(len)
}

#[cfg(test)]
mod tests {
    use super::decode_attribute as d;

    // Expected values follow the HTML standard's tokenizer rules for
    // attribute values (13.2.5.72 to 13.2.5.80) and its named reference table.

    #[test]
    fn plain_text_is_borrowed() {
        assert!(matches!(
            d("echarts"),
            std::borrow::Cow::Borrowed("echarts")
        ));
    }

    #[test]
    fn numeric_references() {
        assert_eq!(d("e&#99;harts"), "echarts");
        assert_eq!(d("&#x31;&#X30;0"), "100");
        assert_eq!(d("&#49"), "1");
        assert_eq!(d("&#0;"), "\u{FFFD}");
        assert_eq!(d("&#xD800;"), "\u{FFFD}");
        assert_eq!(d("&#x110000;"), "\u{FFFD}");
        assert_eq!(d("&#99999999999999999999;"), "\u{FFFD}");
        assert_eq!(d("&#x80;&#x9F;&#x81;"), "\u{20AC}\u{178}\u{81}");
        assert_eq!(d("&#;&#x;&#a"), "&#;&#x;&#a");
    }

    #[test]
    fn named_references() {
        assert_eq!(d("a&amp;b"), "a&b");
        assert_eq!(d("&lt;&gt;&quot;&apos;"), "<>\"'");
        assert_eq!(d("caf&eacute;"), "café");
        // Two code points.
        assert_eq!(d("&NotEqualTilde;"), "\u{2242}\u{338}");
        // Legacy names work without `;`, longest match wins.
        assert_eq!(d("&amp x"), "& x");
        assert_eq!(d("&not it"), "¬ it");
        // The standard's "&notit;" example: in text it decodes to "¬it;", in
        // an attribute `&not` is followed by a letter, so it stays.
        assert_eq!(d("&notit;"), "&notit;");
        // In attributes, a legacy match followed by `=` or alphanumerics stays.
        assert_eq!(d("?a=1&copy=2"), "?a=1&copy=2");
        assert_eq!(d("&ampx"), "&ampx");
        // Unknown names and a bare `&` stay as written.
        assert_eq!(d("&nosuchentity; & &;"), "&nosuchentity; & &;");
        assert_eq!(d("売上&amp;€"), "売上&€");
    }
}
