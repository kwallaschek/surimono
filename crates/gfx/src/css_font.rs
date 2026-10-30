//! Parsing of the CSS `font` shorthand as the canvas `font` setter does it.
//!
//! Grammar (CSS Fonts 4, shorthand form):
//! `[ <style> || small-caps || <weight> || <width> ]? <size> [ / <line-height> ]? <family-list>`
//! or a system font keyword. Behaviour on edge cases follows Chrome's canvas
//! (checked with `tests/chrome/measure_text.mjs` style probes): sizes are
//! clamped to 10000px, `em` and `%` resolve against canvas's 10px default,
//! `rem` against 16px, unterminated strings close at the end of input.

/// Largest font size in CSS px; larger sizes are clamped, as in Chrome.
pub(crate) const MAX_FONT_SIZE_PX: f64 = 10_000.0;

/// Font size the canvas `font` defaults to, and that `em` and `%` refer to.
pub(crate) const CANVAS_DEFAULT_SIZE_PX: f64 = 10.0;

/// Root font size that `rem` refers to.
const ROOT_FONT_SIZE_PX: f64 = 16.0;

/// A generic CSS font family keyword.
///
/// surimono never uses system fonts, so every generic family maps to
/// registered families (see [`crate::FontSet::set_generic_family`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum GenericFamily {
    /// `serif`
    Serif,
    /// `sans-serif`
    SansSerif,
    /// `monospace`
    Monospace,
    /// `cursive`
    Cursive,
    /// `fantasy`
    Fantasy,
    /// `system-ui`
    SystemUi,
    /// `ui-serif`
    UiSerif,
    /// `ui-sans-serif`
    UiSansSerif,
    /// `ui-monospace`
    UiMonospace,
    /// `ui-rounded`
    UiRounded,
    /// `emoji`
    Emoji,
    /// `math`
    Math,
    /// `fangsong`
    Fangsong,
}

impl GenericFamily {
    /// Every generic family, in a fixed order.
    pub const ALL: [GenericFamily; 13] = [
        GenericFamily::Serif,
        GenericFamily::SansSerif,
        GenericFamily::Monospace,
        GenericFamily::Cursive,
        GenericFamily::Fantasy,
        GenericFamily::SystemUi,
        GenericFamily::UiSerif,
        GenericFamily::UiSansSerif,
        GenericFamily::UiMonospace,
        GenericFamily::UiRounded,
        GenericFamily::Emoji,
        GenericFamily::Math,
        GenericFamily::Fangsong,
    ];

    /// The CSS keyword, for example `sans-serif`.
    pub fn keyword(self) -> &'static str {
        match self {
            GenericFamily::Serif => "serif",
            GenericFamily::SansSerif => "sans-serif",
            GenericFamily::Monospace => "monospace",
            GenericFamily::Cursive => "cursive",
            GenericFamily::Fantasy => "fantasy",
            GenericFamily::SystemUi => "system-ui",
            GenericFamily::UiSerif => "ui-serif",
            GenericFamily::UiSansSerif => "ui-sans-serif",
            GenericFamily::UiMonospace => "ui-monospace",
            GenericFamily::UiRounded => "ui-rounded",
            GenericFamily::Emoji => "emoji",
            GenericFamily::Math => "math",
            GenericFamily::Fangsong => "fangsong",
        }
    }

    /// Parses a CSS keyword (ASCII case-insensitive), for example `Sans-Serif`.
    pub fn from_keyword(keyword: &str) -> Option<GenericFamily> {
        GenericFamily::ALL
            .into_iter()
            .find(|g| g.keyword().eq_ignore_ascii_case(keyword))
    }
}

/// One entry of a parsed `font-family` list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FamilyEntry {
    /// A family name, quoted or not, with escapes resolved.
    Named(String),
    /// A generic family keyword (only when unquoted).
    Generic(GenericFamily),
}

/// `font-style`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Slant {
    Normal,
    Italic,
    /// Oblique with an optional angle in degrees.
    Oblique(Option<f32>),
}

/// A parsed CSS `font` shorthand, with the size resolved to CSS px.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FontSpec {
    pub(crate) slant: Slant,
    /// `font-weight`, 1 to 1000.
    pub(crate) weight: f32,
    /// `font-stretch` as a ratio (1.0 is normal).
    pub(crate) width: f32,
    /// `font-size` in CSS px, 0 to [`MAX_FONT_SIZE_PX`].
    pub(crate) size_px: f32,
    pub(crate) families: Vec<FamilyEntry>,
}

impl FontSpec {
    /// Canvas's default font: `10px sans-serif`.
    pub(crate) fn canvas_default() -> FontSpec {
        FontSpec {
            slant: Slant::Normal,
            weight: 400.0,
            width: 1.0,
            size_px: CANVAS_DEFAULT_SIZE_PX as f32,
            families: vec![FamilyEntry::Generic(GenericFamily::SansSerif)],
        }
    }

    /// Parses a CSS `font` shorthand; `None` if the canvas setter would ignore it.
    pub(crate) fn parse(input: &str) -> Option<FontSpec> {
        Self::parse_with_family_offset(input).map(|(spec, _)| spec)
    }

    /// Like [`FontSpec::parse`], also returning the byte offset in `input`
    /// where the family list starts (`None` for system font keywords).
    pub(crate) fn parse_with_family_offset(input: &str) -> Option<(FontSpec, Option<usize>)> {
        let s = trim_ws(input);
        if let Some(spec) = system_font(s) {
            return Some((spec, None));
        }

        let mut slant = None;
        let mut small_caps = false;
        let mut weight = None;
        let mut width = None;
        let mut tokens = 0usize;
        let mut rest = s;

        let size = loop {
            rest = trim_ws_start(rest);
            let (tok, after) = take_token(rest);
            if tok.is_empty() {
                return None;
            }
            let lower = tok.to_ascii_lowercase();
            // Keywords and numbers that come before the size.
            let consumed = if tokens < 4 {
                match lower.as_str() {
                    "normal" => true,
                    "italic" if slant.is_none() => {
                        slant = Some(Slant::Italic);
                        true
                    }
                    "oblique" if slant.is_none() => {
                        // An optional angle may follow.
                        let (next, after_next) = take_token(trim_ws_start(after));
                        if let Some(deg) = parse_angle_deg(next) {
                            slant = Some(Slant::Oblique(Some(deg as f32)));
                            rest = after_next;
                            tokens += 1;
                            continue;
                        }
                        slant = Some(Slant::Oblique(None));
                        true
                    }
                    "small-caps" if !small_caps => {
                        small_caps = true;
                        true
                    }
                    "bold" | "bolder" if weight.is_none() => {
                        // `bolder` and `lighter` are relative to canvas's 400.
                        weight = Some(700.0);
                        true
                    }
                    "lighter" if weight.is_none() => {
                        weight = Some(100.0);
                        true
                    }
                    _ => {
                        if let Some(w) = width_keyword(&lower) {
                            if width.is_some() {
                                return None;
                            }
                            width = Some(w);
                            true
                        } else if let Some(n) = parse_plain_number(tok) {
                            if n == 0.0 {
                                false // unitless zero is a size, not a weight
                            } else if weight.is_none() && (1.0..=1000.0).contains(&n) {
                                weight = Some(n as f32);
                                true
                            } else {
                                return None;
                            }
                        } else {
                            false
                        }
                    }
                }
            } else {
                false
            };
            if consumed {
                tokens += 1;
                rest = after;
                continue;
            }
            // Anything else must be the size.
            let px = parse_size(&lower)?;
            rest = after;
            break px;
        };
        let _ = small_caps; // accepted; Parley has no font-variant, so it does not change shaping

        rest = trim_ws_start(rest);
        if let Some(after_slash) = rest.strip_prefix('/') {
            let (lh, after) = take_token(trim_ws_start(after_slash));
            if !valid_line_height(&lh.to_ascii_lowercase()) {
                return None;
            }
            rest = after;
        }

        let rest = trim_ws_start(rest);
        let families = parse_family_list(rest)?;
        // `rest` is a subslice of `input`; its start is where the families begin.
        let family_offset = rest.as_ptr() as usize - input.as_ptr() as usize;
        // `+ 0.0` turns -0 into +0, so equal sizes have equal bits.
        let size_px = (size.clamp(0.0, MAX_FONT_SIZE_PX) + 0.0) as f32;
        let spec = FontSpec {
            slant: slant.unwrap_or(Slant::Normal),
            weight: weight.unwrap_or(400.0),
            width: width.unwrap_or(1.0),
            size_px,
            families,
        };
        Some((spec, Some(family_offset)))
    }
}

fn is_css_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

fn trim_ws(s: &str) -> &str {
    s.trim_matches(is_css_ws)
}

fn trim_ws_start(s: &str) -> &str {
    s.trim_start_matches(is_css_ws)
}

/// Splits off a token that ends at whitespace, `/`, `,` or a quote.
fn take_token(s: &str) -> (&str, &str) {
    let end = s
        .find(|c: char| is_css_ws(c) || matches!(c, '/' | ',' | '"' | '\''))
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

/// System font keywords; Chrome resolves them to a 16px sans-serif face.
fn system_font(s: &str) -> Option<FontSpec> {
    const KEYWORDS: [&str; 6] = [
        "caption",
        "icon",
        "menu",
        "message-box",
        "small-caption",
        "status-bar",
    ];
    KEYWORDS
        .iter()
        .any(|k| k.eq_ignore_ascii_case(s))
        .then(|| FontSpec {
            size_px: 16.0,
            ..FontSpec::canvas_default()
        })
}

fn width_keyword(lower: &str) -> Option<f32> {
    Some(match lower {
        "ultra-condensed" => 0.5,
        "extra-condensed" => 0.625,
        "condensed" => 0.75,
        "semi-condensed" => 0.875,
        "semi-expanded" => 1.125,
        "expanded" => 1.25,
        "extra-expanded" => 1.5,
        "ultra-expanded" => 2.0,
        _ => return None,
    })
}

/// Parses a CSS number prefix: `[+-]? (digits [. digits?] | . digits) ([eE] [+-]? digits)?`.
/// Returns the value and the unit suffix.
fn split_number(s: &str) -> Option<(f64, &str)> {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - int_start;
    if i < b.len() && b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    let value: f64 = s[..i].parse().ok()?;
    value.is_finite().then_some((value, &s[i..]))
}

fn parse_plain_number(s: &str) -> Option<f64> {
    match split_number(s)? {
        (v, "") => Some(v),
        _ => None,
    }
}

fn parse_angle_deg(s: &str) -> Option<f64> {
    let (v, unit) = split_number(s)?;
    let unit = unit.to_ascii_lowercase();
    let deg = match unit.as_str() {
        "deg" => v,
        "rad" => v.to_degrees(),
        "grad" => v * 0.9,
        "turn" => v * 360.0,
        _ => return None,
    };
    (-90.0..=90.0).contains(&deg).then_some(deg)
}

/// Resolves a `font-size` token (already lower case) to CSS px.
fn parse_size(lower: &str) -> Option<f64> {
    let keyword = match lower {
        "xx-small" => Some(9.0),
        "x-small" => Some(10.0),
        "small" => Some(13.0),
        "medium" => Some(16.0),
        "large" => Some(18.0),
        "x-large" => Some(24.0),
        "xx-large" => Some(32.0),
        "xxx-large" => Some(48.0),
        "larger" => Some(CANVAS_DEFAULT_SIZE_PX * 1.2),
        "smaller" => Some(CANVAS_DEFAULT_SIZE_PX / 1.2),
        _ => None,
    };
    if keyword.is_some() {
        return keyword;
    }
    let px = length_px(lower, CANVAS_DEFAULT_SIZE_PX)?;
    (px >= 0.0).then_some(px)
}

/// Converts a length or percentage token to px. `em` and `%` refer to `em_px`.
fn length_px(lower: &str, em_px: f64) -> Option<f64> {
    let (v, unit) = split_number(lower)?;
    let factor = match unit {
        "" if v == 0.0 => 0.0,
        "px" => 1.0,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "q" => 96.0 / 101.6,
        "em" => em_px,
        "rem" => ROOT_FONT_SIZE_PX,
        // No font is resolved at parse time; CSS's fallback is 0.5em.
        "ex" | "ch" => em_px * 0.5,
        "%" => em_px / 100.0,
        _ => return None,
    };
    let px = v * factor;
    px.is_finite().then_some(px)
}

fn valid_line_height(lower: &str) -> bool {
    if lower == "normal" {
        return true;
    }
    if let Some(n) = parse_plain_number(lower) {
        return n >= 0.0;
    }
    matches!(length_px(lower, CANVAS_DEFAULT_SIZE_PX), Some(px) if px >= 0.0)
}

/// Words an unquoted family name may not use.
fn reserved_family_word(lower: &str) -> bool {
    matches!(
        lower,
        "inherit" | "initial" | "unset" | "revert" | "revert-layer" | "default"
    )
}

/// Parses a comma-separated `font-family` list. Empty entries make it invalid.
pub(crate) fn parse_family_list(s: &str) -> Option<Vec<FamilyEntry>> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        // Skip leading whitespace of the entry.
        while chars.peek().is_some_and(|&c| is_css_ws(c)) {
            chars.next();
        }
        let entry = match chars.peek().copied() {
            None => return None, // empty entry (also "12px" alone, or a trailing comma)
            Some(q @ ('"' | '\'')) => {
                chars.next();
                let mut name = String::new();
                loop {
                    match chars.next() {
                        None => break, // EOF closes the string
                        Some(c) if c == q => break,
                        Some('\\') => {
                            if let Some(c) = read_escape(&mut chars) {
                                name.push(c);
                            }
                        }
                        Some('\n') => return None,
                        Some(c) => name.push(c),
                    }
                }
                FamilyEntry::Named(name)
            }
            Some(_) => {
                let mut words: Vec<String> = Vec::new();
                loop {
                    while chars.peek().is_some_and(|&c| is_css_ws(c)) {
                        chars.next();
                    }
                    match chars.peek() {
                        None | Some(',') => break,
                        _ => {}
                    }
                    words.push(read_ident(&mut chars)?);
                }
                if words.is_empty() {
                    return None;
                }
                if words
                    .iter()
                    .any(|w| reserved_family_word(&w.to_ascii_lowercase()))
                {
                    return None;
                }
                if words.len() == 1 {
                    if let Some(g) = GenericFamily::from_keyword(&words[0]) {
                        out.push(FamilyEntry::Generic(g));
                        if !after_entry(&mut chars)? {
                            return Some(out);
                        }
                        continue;
                    }
                }
                FamilyEntry::Named(words.join(" "))
            }
        };
        out.push(entry);
        if !after_entry(&mut chars)? {
            return Some(out);
        }
    }
}

/// After an entry: skips whitespace, then expects a comma (returns true) or
/// the end (returns false). Anything else is invalid.
fn after_entry(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<bool> {
    while chars.peek().is_some_and(|&c| is_css_ws(c)) {
        chars.next();
    }
    match chars.next() {
        None => Some(false),
        Some(',') => Some(true),
        Some(_) => None,
    }
}

/// Reads one CSS identifier. Fails on characters an identifier can't hold.
fn read_ident(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<String> {
    let mut out = String::new();
    let mut first = true;
    while let Some(&c) = chars.peek() {
        if is_css_ws(c) || c == ',' {
            break;
        }
        chars.next();
        let ch = if c == '\\' {
            read_escape(chars)?
        } else if c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii() {
            c
        } else {
            return None;
        };
        if first {
            // An identifier can't start with a digit, or with '-' then a digit.
            if c != '\\' && ch.is_ascii_digit() {
                return None;
            }
            first = false;
        }
        out.push(ch);
    }
    let b = out.as_bytes();
    if b.first() == Some(&b'-') && b.get(1).is_some_and(|d| d.is_ascii_digit()) {
        return None;
    }
    (!out.is_empty()).then_some(out)
}

/// Reads a CSS escape after the backslash: 1 to 6 hex digits and an optional
/// whitespace, or any other single character.
fn read_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<char> {
    let mut hex = String::new();
    while hex.len() < 6 && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
        hex.push(chars.next()?);
    }
    if hex.is_empty() {
        return match chars.next() {
            Some('\n') | None => None,
            Some(c) => Some(c),
        };
    }
    if chars.peek().is_some_and(|&c| is_css_ws(c)) {
        chars.next();
    }
    let cp = u32::from_str_radix(&hex, 16).ok()?;
    Some(match char::from_u32(cp) {
        Some(c) if cp != 0 => c,
        _ => char::REPLACEMENT_CHARACTER,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(s: &str) -> FamilyEntry {
        FamilyEntry::Named(s.to_string())
    }

    fn generic(g: GenericFamily) -> FamilyEntry {
        FamilyEntry::Generic(g)
    }

    fn parse(s: &str) -> FontSpec {
        FontSpec::parse(s).unwrap_or_else(|| panic!("{s:?} should parse"))
    }

    #[test]
    fn brief_examples() {
        let a = parse("12px sans-serif");
        assert_eq!(a.size_px, 12.0);
        assert_eq!(a.families, vec![generic(GenericFamily::SansSerif)]);

        let b = parse("bold 14px 'Noto Sans JP', sans-serif");
        assert_eq!(b.weight, 700.0);
        assert_eq!(b.size_px, 14.0);
        assert_eq!(
            b.families,
            vec![named("Noto Sans JP"), generic(GenericFamily::SansSerif)]
        );

        let c = parse("italic 600 13.5px/1.2 Arial");
        assert_eq!(c.slant, Slant::Italic);
        assert_eq!(c.weight, 600.0);
        assert_eq!(c.size_px, 13.5);
        assert_eq!(c.families, vec![named("Arial")]);
    }

    /// Each case: input, then Chrome's canvas `ctx.font` after setting it
    /// (Chromium 153 via Playwright 1.63), reduced to size in px or invalid.
    /// Probed with a detached canvas; "INVALID" means Chrome kept the old font.
    #[test]
    fn chrome_validity_and_sizes() {
        let cases: &[(&str, Option<f32>)] = &[
            ("1rem x", Some(16.0)),
            ("larger x", Some(12.0)),
            ("smaller x", Some(10.0 / 1.2)),
            ("medium x", Some(16.0)),
            ("xx-small x", Some(9.0)),
            ("xxx-large x", Some(48.0)),
            ("0 x", Some(0.0)),
            ("0px x", Some(0.0)),
            ("oblique 10deg 12px x", Some(12.0)),
            ("oblique 12px x", Some(12.0)),
            ("bolder 12px x", Some(12.0)),
            ("lighter 12px x", Some(12.0)),
            ("small-caps 12px x", Some(12.0)),
            ("normal normal normal normal 12px x", Some(12.0)),
            ("normal normal normal normal normal 12px x", None),
            ("bold bold 12px x", None),
            ("600 12px x", Some(12.0)),
            ("1000 12px x", Some(12.0)),
            ("1001 12px x", None),
            ("0.5 12px x", None),
            ("condensed 12px x", Some(12.0)),
            ("ultra-expanded 12px x", Some(12.0)),
            ("12px", None),
            ("12px inherit", None),
            ("inherit", None),
            ("12px/normal x", Some(12.0)),
            ("12px / 1.5 x", Some(12.0)),
            ("12px/-1 x", None),
            ("-12px x", None),
            ("12px 3D", None),
            ("12px foo bar", Some(12.0)),
            ("12px \"a b\", c", Some(12.0)),
            ("12px \"unterminated", Some(12.0)),
            ("12px a,,b", None),
            ("12px a,", None),
            ("12px default", None),
            ("12PX SANS-SERIF", Some(12.0)),
            ("BOLD 12px x", Some(12.0)),
            ("bold italic 12px x", Some(12.0)),
            ("12.5e1px x", Some(125.0)),
            ("1e1px x", Some(10.0)),
            (".5em x", Some(5.0)),
            ("+12px x", Some(12.0)),
            ("12px\tx", Some(12.0)),
            ("12px\nx", Some(12.0)),
            ("caption", Some(16.0)),
            ("12px \\66oo", Some(12.0)),
            ("12px math", Some(12.0)),
            ("12px initial", None),
            ("100% x", Some(10.0)),
            ("12px/1.2px x", Some(12.0)),
            ("12px/50% x", Some(12.0)),
            ("semi-condensed bold italic small-caps 12px x", Some(12.0)),
            ("italic italic 12px x", None),
            ("1e6px x", Some(10000.0)),
            ("12px x !important", None),
            ("12 px x", None),
            ("12pxx x", None),
            ("9pt 'Noto Sans'", Some(12.0)),
            ("1.4em 'Noto Sans'", Some(14.0)),
            ("150% 'Noto Sans'", Some(15.0)),
        ];
        for (input, want) in cases {
            let got = FontSpec::parse(input).map(|s| s.size_px);
            match (got, want) {
                (Some(g), Some(w)) => assert!((g - w).abs() < 1e-4, "{input:?}: {g} != {w}"),
                (None, None) => {}
                _ => panic!("{input:?}: got {got:?}, Chrome says {want:?}"),
            }
        }
    }

    #[test]
    fn family_names() {
        assert_eq!(parse("12px  foo   bar ").families, vec![named("foo bar")]);
        assert_eq!(
            parse("12px x y, \"z\"").families,
            vec![named("x y"), named("z")]
        );
        assert_eq!(parse("12px \\66oo").families, vec![named("foo")]);
        assert_eq!(
            parse("12px serif, \"sans-serif\"").families,
            vec![generic(GenericFamily::Serif), named("sans-serif")]
        );
        assert_eq!(
            parse("12px 'it\\'s'").families,
            vec![named("it's")],
            "escaped quote inside a string"
        );
        assert_eq!(
            parse("12px system-ui, emoji").families,
            vec![
                generic(GenericFamily::SystemUi),
                generic(GenericFamily::Emoji)
            ]
        );
    }

    #[test]
    fn family_offset_points_at_the_list() {
        for (input, list) in [
            ("12px x", "x"),
            ("  bold 14px/1.2 'A b', serif ", "'A b', serif "),
            ("italic 600 13.5px/1.2 Arial", "Arial"),
            ("12px\tx", "x"),
        ] {
            let (_, off) = FontSpec::parse_with_family_offset(input).expect("parses");
            assert_eq!(&input[off.expect("offset")..], list, "{input:?}");
        }
        assert_eq!(
            FontSpec::parse_with_family_offset("caption").map(|p| p.1),
            Some(None)
        );
    }

    #[test]
    fn weights_and_styles() {
        assert_eq!(parse("bolder 12px x").weight, 700.0);
        assert_eq!(parse("lighter 12px x").weight, 100.0);
        assert_eq!(parse("650.5 12px x").weight, 650.5);
        assert_eq!(parse("condensed 12px x").width, 0.75);
        assert_eq!(
            parse("oblique 10deg 12px x").slant,
            Slant::Oblique(Some(10.0))
        );
        assert_eq!(parse("oblique 12px x").slant, Slant::Oblique(None));
    }

    #[test]
    fn hostile_input_never_panics() {
        let long = "a".repeat(100_000);
        let inputs = [
            "",
            " ",
            "/",
            "//",
            "12px/",
            "12px/ x",
            "\\",
            "12px \\",
            "12px '\\",
            "12px \"\\\n\"",
            "12px \\0",
            "12px \\110000",
            "1e999px x",
            "-0px x",
            "NaNpx x",
            "infinitypx x",
            "12px ,",
            "12px 'a' 'b'",
            "12px 'a' b",
            "\u{0}",
            "12px 日本語",
            "１２px x",
            long.as_str(),
        ];
        for s in inputs {
            let _ = FontSpec::parse(s);
        }
        assert!(FontSpec::parse("1e999px x").is_none());
        assert_eq!(parse("12px 日本語").families, vec![named("日本語")]);
        assert_eq!(parse("-0px x").size_px, 0.0);
    }
}
