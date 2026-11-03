//! The `style` attribute as `CSSStyleDeclaration` sees it (CSSOM §6.6),
//! kept as text in the attribute, which is what gets serialized and printed.
//!
//! Declarations are split, not validated: an invalid value or a shorthand
//! stays as written, where a browser would drop or expand it. The printing
//! engine parses the attribute itself and ignores what it can't use.

use style::properties::PropertyId;

/// One `name: value` of a style attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declaration {
    pub name: String,
    pub value: String,
    pub important: bool,
}

/// Lower-cases `property`, unless it is a custom property.
pub(crate) fn normalized(property: &str) -> String {
    if property.starts_with("--") {
        property.to_owned()
    } else {
        property.to_ascii_lowercase()
    }
}

/// The CSS property for a `CSSStyleProperties` attribute name (CSSOM §6.7):
/// `cssFloat`, `fontSize`, `webkitFoo`, or the dashed `font-size`. `None` if
/// that isn't a known property.
pub(crate) fn property_for_attribute(attribute: &str) -> Option<String> {
    let dashed = if attribute == "cssFloat" {
        "float".to_owned()
    } else if attribute.contains('-') {
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
    PropertyId::parse_enabled_for_all_content(&dashed)
        .ok()
        .map(|_| dashed)
}

/// Splits `text` at top-level `;`, skipping strings, brackets and comments.
fn split_declarations(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut quote = None;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match (quote, c) {
            (Some(_), '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth = depth.saturating_sub(1),
            (None, ';') if depth == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

/// Parses a style attribute's declarations; a later one of the same name
/// replaces an earlier one.
pub(crate) fn parse(text: &str) -> Vec<Declaration> {
    let mut list = Vec::new();
    for part in split_declarations(text) {
        let Some((name, value)) = part.split_once(':') else {
            continue;
        };
        let (name, mut value) = (name.trim(), value.trim());
        if name.is_empty() || value.is_empty() {
            continue;
        }
        let mut important = false;
        if let Some(head) = value
            .len()
            .checked_sub("important".len())
            .filter(|&at| value.is_char_boundary(at))
            .map(|at| value.split_at(at))
            .filter(|(_, tail)| tail.eq_ignore_ascii_case("important"))
            .and_then(|(head, _)| head.trim_end().strip_suffix('!'))
        {
            value = head.trim_end();
            important = true;
        }
        set(&mut list, normalized(name), value.to_owned(), important);
    }
    list
}

/// Sets `name`, in place if already there.
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
        out.push_str(&d.name);
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

    #[test]
    fn parses_as_css_does() {
        let list = parse(
            " color : red ; background: url(a;b.png) ; content: 'x;y'; margin: 0 ! important;;bad; x:",
        );
        let got: Vec<_> = list
            .iter()
            .map(|d| (d.name.as_str(), d.value.as_str(), d.important))
            .collect();
        assert_eq!(
            got,
            [
                ("color", "red", false),
                ("background", "url(a;b.png)", false),
                ("content", "'x;y'", false),
                ("margin", "0", true),
            ]
        );
        assert_eq!(serialize(&list[..1]), "color: red;");
        assert_eq!(serialize(&list[3..]), "margin: 0 !important;");
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
        ] {
            let _ = parse(s);
        }
    }

    #[test]
    fn attribute_names() {
        for (attr, property) in [
            ("fontSize", Some("font-size")),
            ("cssFloat", Some("float")),
            ("font-size", Some("font-size")),
            ("color", Some("color")),
            ("notAProperty", None),
            ("length", None),
        ] {
            assert_eq!(property_for_attribute(attr).as_deref(), property, "{attr}");
        }
    }
}
