//! `matchMedia` (CSSOM View §4.2): a media query list parsed as Media Queries 4
//! says, serialized as CSSOM §4.2 says, and evaluated for the window a script
//! sees.
//!
//! Stylo's own parser is reachable, but in its Servo configuration it knows six
//! features (no `orientation`, `aspect-ratio`, `hover`, `pointer`, `color`...),
//! and a query it doesn't know becomes `not all`, where Chromium keeps the
//! text. So the list is parsed here, over cssparser's tokens, for the features
//! that need no layout; every other feature is `<general-enclosed>`, which MQ4
//! keeps as written and evaluates to "unknown".
//!
//! Bounded: the input is capped at [`MAX_QUERY`] (past it the list is
//! `not all`), the grammar nests at most [`MAX_NESTING`] brackets (deeper, the
//! query is `not all`), and everything else is one pass over the tokens.

use cssparser::{Delimiter, ParseError, Parser, ParserInput, Token};

use crate::sheet_screen::MAX_NESTING;

/// Most UTF-16 code units of a query list, checked in the glue before the text
/// crosses, and most bytes here. Media queries are short (Bootstrap's longest
/// is about 100 characters, a list of a hundred breakpoints about 4 KiB), and a
/// parse can't be interrupted. Past it: `not all`, matching nothing.
pub(crate) const MAX_QUERY: usize = 64 << 10;

/// What a media query is evaluated against: the window a script sees.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Env {
    /// `innerWidth` and `innerHeight`, in CSS pixels.
    pub(crate) width: f64,
    pub(crate) height: f64,
    /// `devicePixelRatio`.
    pub(crate) dpr: f64,
}

/// A parsed and evaluated list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Evaluated {
    /// CSSOM's serialization, the `media` attribute.
    pub(crate) text: String,
    pub(crate) matches: bool,
    /// Whether the answer read the viewport's size, which is a layout query.
    pub(crate) sized: bool,
}

/// Parses `query` as a `<media-query-list>` and evaluates it in `env`.
pub(crate) fn match_media(query: &str, env: &Env) -> Evaluated {
    if query.len() > MAX_QUERY {
        return Evaluated {
            text: "not all".to_owned(),
            matches: false,
            sized: false,
        };
    }
    let list = parse_list(query);
    let mut text = String::new();
    let mut matches = list.is_empty();
    let mut ev = Eval { env, sized: false };
    for (i, q) in list.iter().enumerate() {
        if i > 0 {
            text.push_str(", ");
        }
        match q {
            Some(q) => {
                q.serialize(&mut text);
                matches |= ev.query(q) == Some(true);
            }
            None => text.push_str("not all"),
        }
    }
    Evaluated {
        text,
        matches,
        sized: ev.sized,
    }
}

type Failed<'i> = ParseError<'i, ()>;

/// `None` for a query that failed to parse: `not all`.
fn parse_list(text: &str) -> Vec<Option<Query>> {
    let mut input = ParserInput::new(text);
    let mut p = Parser::new(&mut input);
    p.skip_whitespace();
    if p.is_exhausted() {
        return Vec::new();
    }
    let mut out = Vec::new();
    loop {
        p.skip_whitespace();
        out.push(p.parse_until_before(Delimiter::Comma, |p| query(p)).ok());
        match p.next() {
            Ok(Token::Comma) => {}
            _ => return out,
        }
    }
}

/// `<media-query>`.
#[derive(Debug)]
enum Query {
    /// `[not | only]? <media-type> [and <media-condition-without-or>]?`
    Typed {
        not: bool,
        only: bool,
        ty: String,
        condition: Option<Condition>,
    },
    Condition(Condition),
}

/// `<media-condition>`.
#[derive(Debug)]
enum Condition {
    Not(Box<Condition>),
    And(Vec<Condition>),
    Or(Vec<Condition>),
    /// `( <media-condition> )`
    Parens(Box<Condition>),
    Feature(Feature),
    /// `<general-enclosed>`, as written.
    Enclosed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Op {
    fn text(self) -> &'static str {
        match self {
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Eq => "=",
        }
    }

    /// `a op b` read the other way round: `b op' a`.
    fn flipped(self) -> Op {
        match self {
            Op::Lt => Op::Gt,
            Op::Le => Op::Ge,
            Op::Gt => Op::Lt,
            Op::Ge => Op::Le,
            Op::Eq => Op::Eq,
        }
    }
}

/// `<media-feature>`, with a feature this module knows.
#[derive(Debug)]
enum Feature {
    /// `(name)`
    Boolean(&'static Known),
    /// `(name: value)`, `(min-name: value)`, `(max-name: value)`
    Plain {
        known: &'static Known,
        prefix: Prefix,
        value: Value,
    },
    /// `(name op value)` or `(value op name)`, kept as written for the
    /// serialization; `(v1 op1 name op2 v2)` as two.
    Range {
        known: &'static Known,
        /// The value before the name and its operator, if any.
        before: Option<(Value, Op)>,
        /// The operator and value after the name, if any.
        after: Option<(Op, Value)>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prefix {
    None,
    Min,
    Max,
}

/// A feature value as written: what it serializes as and what it is.
#[derive(Debug, Clone)]
enum Value {
    /// A length in CSS pixels, with its text; for a viewport unit, the
    /// number of hundredths of that axis instead.
    Length(f64, String, Option<Axis>),
    /// A resolution in dppx, with its text.
    Resolution(f64, String),
    /// A plain number (integers included).
    Number(f64),
    Ratio(f64, f64),
    Keyword(String),
}

impl Value {
    fn serialize(&self, out: &mut String) {
        match self {
            Value::Length(_, t, _) | Value::Resolution(_, t) => out.push_str(t),
            Value::Number(n) => out.push_str(&number(*n)),
            Value::Ratio(a, b) => {
                out.push_str(&number(*a));
                out.push_str(" / ");
                out.push_str(&number(*b));
            }
            Value::Keyword(k) => out.push_str(k),
        }
    }
}

/// What a feature's values are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `<length>`, a range feature.
    Length,
    /// `<ratio>`, a range feature.
    Ratio,
    /// `<resolution>`, a range feature.
    Resolution,
    /// A non-negative `<integer>`, a range feature.
    Integer,
    /// A `<number>`, a range feature (`-webkit-device-pixel-ratio`).
    Number,
    /// `<mq-boolean>`: 0 or 1, discrete.
    Bit,
    /// One of the keywords, discrete.
    Keyword(&'static [&'static str]),
}

impl Kind {
    fn range(self) -> bool {
        !matches!(self, Kind::Bit | Kind::Keyword(_))
    }
}

/// What a known feature answers.
#[derive(Debug, Clone, Copy)]
enum Answer {
    Width,
    Height,
    AspectRatio,
    Orientation,
    Resolution,
    /// A fixed integer.
    Int(i32),
    /// A fixed keyword; `"none"` is false in a boolean context, as is each
    /// `no-preference`.
    Word(&'static str),
}

/// A media feature this module evaluates.
#[derive(Debug)]
struct Known {
    name: &'static str,
    kind: Kind,
    answer: Answer,
}

const fn known(name: &'static str, kind: Kind, answer: Answer) -> Known {
    Known { name, kind, answer }
}

/// The features of Media Queries 4 and 5 that need no layout, and what they
/// answer for a page that is printed, not used: no pointing device and no
/// hover (DECISIONS 15 Nov, no interactive capabilities), an sRGB screen of 8
/// bits per channel, light, and no preference about motion, which is what
/// headless Chromium answers and what the reference renders saw. The device
/// is the window: `device-width` is `width`.
static FEATURES: &[Known] = &[
    known(
        "any-hover",
        Kind::Keyword(&["none", "hover"]),
        Answer::Word("none"),
    ),
    known(
        "any-pointer",
        Kind::Keyword(&["none", "coarse", "fine"]),
        Answer::Word("none"),
    ),
    known("aspect-ratio", Kind::Ratio, Answer::AspectRatio),
    known("color", Kind::Integer, Answer::Int(8)),
    known(
        "color-gamut",
        Kind::Keyword(&["srgb", "p3", "rec2020"]),
        Answer::Word("srgb"),
    ),
    known("color-index", Kind::Integer, Answer::Int(0)),
    known("device-aspect-ratio", Kind::Ratio, Answer::AspectRatio),
    known("device-height", Kind::Length, Answer::Height),
    known("device-width", Kind::Length, Answer::Width),
    known(
        "display-mode",
        Kind::Keyword(&[
            "fullscreen",
            "standalone",
            "minimal-ui",
            "browser",
            "picture-in-picture",
        ]),
        Answer::Word("browser"),
    ),
    known(
        "dynamic-range",
        Kind::Keyword(&["standard", "high"]),
        Answer::Word("standard"),
    ),
    known(
        "forced-colors",
        Kind::Keyword(&["none", "active"]),
        Answer::Word("none"),
    ),
    known("grid", Kind::Bit, Answer::Int(0)),
    known("height", Kind::Length, Answer::Height),
    known(
        "hover",
        Kind::Keyword(&["none", "hover"]),
        Answer::Word("none"),
    ),
    known("monochrome", Kind::Integer, Answer::Int(0)),
    known(
        "orientation",
        Kind::Keyword(&["portrait", "landscape"]),
        Answer::Orientation,
    ),
    known(
        "overflow-block",
        Kind::Keyword(&["none", "scroll", "paged"]),
        Answer::Word("scroll"),
    ),
    known(
        "overflow-inline",
        Kind::Keyword(&["none", "scroll"]),
        Answer::Word("scroll"),
    ),
    known(
        "pointer",
        Kind::Keyword(&["none", "coarse", "fine"]),
        Answer::Word("none"),
    ),
    known(
        "prefers-color-scheme",
        Kind::Keyword(&["light", "dark"]),
        Answer::Word("light"),
    ),
    known(
        "prefers-contrast",
        Kind::Keyword(&["no-preference", "less", "more", "custom"]),
        Answer::Word("no-preference"),
    ),
    known(
        "prefers-reduced-motion",
        Kind::Keyword(&["no-preference", "reduce"]),
        Answer::Word("no-preference"),
    ),
    known(
        "prefers-reduced-transparency",
        Kind::Keyword(&["no-preference", "reduce"]),
        Answer::Word("no-preference"),
    ),
    known("resolution", Kind::Resolution, Answer::Resolution),
    known(
        "scan",
        Kind::Keyword(&["interlace", "progressive"]),
        Answer::Word(""),
    ),
    known(
        "scripting",
        Kind::Keyword(&["none", "initial-only", "enabled"]),
        Answer::Word("enabled"),
    ),
    known(
        "update",
        Kind::Keyword(&["none", "slow", "fast"]),
        Answer::Word("fast"),
    ),
    known("width", Kind::Length, Answer::Width),
];

/// `-webkit-device-pixel-ratio` and its prefixed forms (Compat §3.2), a
/// `<number>` range feature whose prefix comes after `-webkit-`.
static WEBKIT_DPR: Known = known(
    "-webkit-device-pixel-ratio",
    Kind::Number,
    Answer::Resolution,
);

/// The known feature `name` (ASCII lower case) stands for, with its prefix.
fn lookup(name: &str) -> Option<(&'static Known, Prefix)> {
    if let Some(rest) = name.strip_prefix("-webkit-") {
        return match rest {
            "device-pixel-ratio" => Some((&WEBKIT_DPR, Prefix::None)),
            "min-device-pixel-ratio" => Some((&WEBKIT_DPR, Prefix::Min)),
            "max-device-pixel-ratio" => Some((&WEBKIT_DPR, Prefix::Max)),
            _ => None,
        };
    }
    let (prefix, base) = if let Some(b) = name.strip_prefix("min-") {
        (Prefix::Min, b)
    } else if let Some(b) = name.strip_prefix("max-") {
        (Prefix::Max, b)
    } else {
        (Prefix::None, name)
    };
    let k = FEATURES
        .binary_search_by(|k| k.name.cmp(base))
        .ok()
        .map(|i| &FEATURES[i])?;
    Some((k, prefix))
}

fn ident_is(t: &Token<'_>, word: &str) -> bool {
    matches!(t, Token::Ident(i) if i.eq_ignore_ascii_case(word))
}

/// `<media-query>`.
fn query<'i>(p: &mut Parser<'i, '_>) -> Result<Query, Failed<'i>> {
    if let Ok(c) = p.try_parse(|p| condition(p, true, 0)) {
        return Ok(Query::Condition(c));
    }
    let mut not = false;
    let mut only = false;
    let mut ty = p.expect_ident()?.to_ascii_lowercase();
    if ty == "not" || ty == "only" {
        not = ty == "not";
        only = ty == "only";
        ty = p.expect_ident()?.to_ascii_lowercase();
    }
    if matches!(ty.as_str(), "only" | "not" | "and" | "or" | "layer") {
        return Err(p.new_custom_error(()));
    }
    if p.is_exhausted() {
        return Ok(Query::Typed {
            not,
            only,
            ty,
            condition: None,
        });
    }
    let and = p.next()?.clone();
    if !ident_is(&and, "and") {
        return Err(p.new_custom_error(()));
    }
    let c = condition(p, false, 0)?;
    Ok(Query::Typed {
        not,
        only,
        ty,
        condition: Some(c),
    })
}

/// `<media-condition>`, or `<media-condition-without-or>` without `or_ok`,
/// to the end of `p`.
fn condition<'i>(
    p: &mut Parser<'i, '_>,
    or_ok: bool,
    depth: usize,
) -> Result<Condition, Failed<'i>> {
    if p.try_parse(|p| p.expect_ident_matching("not")).is_ok() {
        let inner = in_parens(p, depth)?;
        p.expect_exhausted()?;
        return Ok(Condition::Not(Box::new(inner)));
    }
    let first = in_parens(p, depth)?;
    if p.is_exhausted() {
        return Ok(first);
    }
    let word = match p.next()? {
        Token::Ident(w) if w.eq_ignore_ascii_case("and") => "and",
        Token::Ident(w) if or_ok && w.eq_ignore_ascii_case("or") => "or",
        _ => return Err(p.new_custom_error(())),
    };
    let mut list = vec![first];
    loop {
        list.push(in_parens(p, depth)?);
        if p.is_exhausted() {
            break;
        }
        p.expect_ident_matching(word)?;
    }
    Ok(if word == "and" {
        Condition::And(list)
    } else {
        Condition::Or(list)
    })
}

/// `<media-in-parens>`.
fn in_parens<'i>(p: &mut Parser<'i, '_>, depth: usize) -> Result<Condition, Failed<'i>> {
    if depth >= MAX_NESTING {
        return Err(p.new_custom_error(()));
    }
    p.skip_whitespace();
    let start = p.position();
    match p.next()? {
        Token::ParenthesisBlock => {}
        Token::Function(_) => {
            p.parse_nested_block(|p| any_value(p, depth + 1))?;
            return Ok(Condition::Enclosed(p.slice_from(start).to_owned()));
        }
        _ => return Err(p.new_custom_error(())),
    }
    let parsed = p.parse_nested_block(|p| {
        if let Ok(c) = p.try_parse(|p| condition(p, true, depth + 1)) {
            return Ok(Some(Condition::Parens(Box::new(c))));
        }
        if let Ok(f) = p.try_parse(feature) {
            return Ok(Some(Condition::Feature(f)));
        }
        any_value(p, depth + 1)?;
        Ok(None)
    })?;
    Ok(parsed.unwrap_or_else(|| Condition::Enclosed(p.slice_from(start).to_owned())))
}

/// `<any-value>?` to the end of `p`: no error token, at most [`MAX_NESTING`]
/// brackets deep.
fn any_value<'i>(p: &mut Parser<'i, '_>, depth: usize) -> Result<(), Failed<'i>> {
    loop {
        let t = match p.next_including_whitespace_and_comments() {
            Ok(t) => t.clone(),
            Err(_) => return Ok(()),
        };
        match t {
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                if depth + 1 >= MAX_NESTING {
                    return Err(p.new_custom_error(()));
                }
                p.parse_nested_block(|p| any_value(p, depth + 1))?;
            }
            t if t.is_parse_error() => return Err(p.new_custom_error(())),
            _ => {}
        }
    }
}

/// `<media-feature>` of a known feature, the whole of `p`.
fn feature<'i>(p: &mut Parser<'i, '_>) -> Result<Feature, Failed<'i>> {
    // The name first: `(name)`, `(name: value)`, `(name op value)`.
    if let Ok(f) = p.try_parse(|p| {
        let name = p.expect_ident()?.to_ascii_lowercase();
        let Some((known, prefix)) = lookup(&name) else {
            return Err(p.new_custom_error(()));
        };
        if p.is_exhausted() {
            return if prefix == Prefix::None {
                Ok(Feature::Boolean(known))
            } else {
                Err(p.new_custom_error(()))
            };
        }
        if p.try_parse(|p| p.expect_colon()).is_ok() {
            if prefix != Prefix::None && !known.kind.range() {
                return Err(p.new_custom_error(()));
            }
            let value = value(p, known.kind)?;
            p.expect_exhausted()?;
            return Ok(Feature::Plain {
                known,
                prefix,
                value,
            });
        }
        if prefix != Prefix::None || !known.kind.range() {
            return Err(p.new_custom_error(()));
        }
        let op = operator(p)?;
        let v = value(p, known.kind)?;
        p.expect_exhausted()?;
        Ok(Feature::Range {
            known,
            before: None,
            after: Some((op, v)),
        })
    }) {
        return Ok(f);
    }
    // `(value op name)` and `(value op name op value)`. The value's kind is
    // known only once the name is, so it is read twice.
    let start = p.state();
    let skipped = skip_value(p)?;
    let op1 = operator(p)?;
    let name = p.expect_ident()?.to_ascii_lowercase();
    let Some((known, Prefix::None)) = lookup(&name) else {
        return Err(p.new_custom_error(()));
    };
    if !known.kind.range() {
        return Err(p.new_custom_error(()));
    }
    let after = if p.is_exhausted() {
        None
    } else {
        let op2 = operator(p)?;
        let lt = |o| matches!(o, Op::Lt | Op::Le);
        let gt = |o| matches!(o, Op::Gt | Op::Ge);
        if !(lt(op1) && lt(op2) || gt(op1) && gt(op2)) {
            return Err(p.new_custom_error(()));
        }
        let v = value(p, known.kind)?;
        p.expect_exhausted()?;
        Some((op2, v))
    };
    let end = p.state();
    p.reset(&start);
    let v1 = value(p, known.kind)?;
    let _ = skipped;
    p.reset(&end);
    Ok(Feature::Range {
        known,
        before: Some((v1, op1)),
        after,
    })
}

/// Steps over what [`value`] would read, without knowing its kind: a token,
/// or `a / b`.
fn skip_value<'i>(p: &mut Parser<'i, '_>) -> Result<(), Failed<'i>> {
    match p.next()? {
        Token::Number { .. } | Token::Dimension { .. } => {}
        _ => return Err(p.new_custom_error(())),
    }
    let _ = p.try_parse(|p| -> Result<(), Failed<'i>> {
        p.expect_delim('/')?;
        match p.next()? {
            Token::Number { .. } => Ok(()),
            _ => Err(p.new_custom_error(())),
        }
    });
    Ok(())
}

/// `<mf-lt>`, `<mf-gt>` or `<mf-eq>`: `<=` and `>=` are two delimiters with
/// nothing between them.
fn operator<'i>(p: &mut Parser<'i, '_>) -> Result<Op, Failed<'i>> {
    let first = match p.next()? {
        Token::Delim('<') => Op::Lt,
        Token::Delim('>') => Op::Gt,
        Token::Delim('=') => return Ok(Op::Eq),
        _ => return Err(p.new_custom_error(())),
    };
    let eq = p.try_parse(|p| -> Result<(), Failed<'i>> {
        match p.next_including_whitespace()? {
            Token::Delim('=') => Ok(()),
            _ => Err(p.new_custom_error(())),
        }
    });
    Ok(match (first, eq.is_ok()) {
        (Op::Lt, true) => Op::Le,
        (Op::Gt, true) => Op::Ge,
        (op, _) => op,
    })
}

/// A finite `f32` as cssparser gives it, out-of-range values at the largest
/// one (Chromium serializes `1e400px` as `3.40282e+38px`).
fn finite(v: f32) -> f64 {
    f64::from(if v.is_finite() {
        v
    } else if v > 0.0 {
        f32::MAX
    } else {
        f32::MIN
    })
}

/// A value of `kind`, the next token or tokens of `p`.
fn value<'i>(p: &mut Parser<'i, '_>, kind: Kind) -> Result<Value, Failed<'i>> {
    let t = p.next()?.clone();
    let bad = |p: &Parser<'i, '_>| p.new_custom_error(());
    match kind {
        Kind::Length => match t {
            Token::Number { value, .. } if value == 0.0 => {
                Ok(Value::Length(0.0, "0".to_owned(), None))
            }
            Token::Dimension { value, unit, .. } => {
                let unit = unit.to_ascii_lowercase();
                let n = finite(value);
                let text = format!("{}{unit}", number(n));
                if let Some(axis) = viewport_unit(&unit) {
                    return Ok(Value::Length(n, text, Some(axis)));
                }
                let px = length_px(n, &unit).ok_or_else(|| bad(p))?;
                Ok(Value::Length(px, text, None))
            }
            _ => Err(bad(p)),
        },
        Kind::Resolution => match t {
            Token::Dimension { value, unit, .. } if value >= 0.0 => {
                let unit = unit.to_ascii_lowercase();
                let n = finite(value);
                let dppx = match unit.as_str() {
                    "dppx" | "x" => n,
                    "dpi" => n / 96.0,
                    "dpcm" => n * 2.54 / 96.0,
                    _ => return Err(bad(p)),
                };
                Ok(Value::Resolution(dppx, format!("{}{unit}", number(n))))
            }
            _ => Err(bad(p)),
        },
        Kind::Integer => match t {
            Token::Number {
                int_value: Some(i), ..
            } if i >= 0 => Ok(Value::Number(f64::from(i))),
            _ => Err(bad(p)),
        },
        Kind::Bit => match t {
            Token::Number {
                int_value: Some(i @ (0 | 1)),
                ..
            } => Ok(Value::Number(f64::from(i))),
            _ => Err(bad(p)),
        },
        Kind::Number => match t {
            Token::Number { value, .. } => Ok(Value::Number(finite(value))),
            _ => Err(bad(p)),
        },
        Kind::Ratio => {
            let a = match t {
                Token::Number { value, .. } if value >= 0.0 => finite(value),
                _ => return Err(bad(p)),
            };
            let b = p
                .try_parse(|p| -> Result<f64, Failed<'i>> {
                    p.expect_delim('/')?;
                    match p.next()? {
                        Token::Number { value, .. } if *value >= 0.0 => Ok(finite(*value)),
                        _ => Err(p.new_custom_error(())),
                    }
                })
                .unwrap_or(1.0);
            Ok(Value::Ratio(a, b))
        }
        Kind::Keyword(words) => match t {
            Token::Ident(w) => {
                let w = w.to_ascii_lowercase();
                if words.contains(&w.as_str()) {
                    Ok(Value::Keyword(w))
                } else {
                    Err(bad(p))
                }
            }
            _ => Err(bad(p)),
        },
    }
}

/// `n` of `unit` in CSS pixels, for a media query (MQ4 §1.3: relative units
/// against the initial values, a 16px font; css-values-4's fallbacks for the
/// font's metrics, which a query can't measure: `ex` and `ch` are 0.5em, `ic`
/// and `cap` 1em, `lh` 1.2em). `None` for no length unit.
fn length_px(n: f64, unit: &str) -> Option<f64> {
    const EM: f64 = 16.0;
    let per = match unit {
        "px" => 1.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "q" => 96.0 / 101.6,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "em" | "rem" | "ic" | "ric" | "cap" | "rcap" => EM,
        "ex" | "rex" | "ch" | "rch" => EM / 2.0,
        "lh" | "rlh" => EM * 1.2,
        _ => return None,
    };
    Some(n * per)
}

/// The axis of a viewport unit (css-values-4 §6.1.2; container units fall
/// back to the small viewport's), `None` for another unit.
fn viewport_unit(unit: &str) -> Option<Axis> {
    let base = unit
        .strip_prefix("sv")
        .or_else(|| unit.strip_prefix("lv"))
        .or_else(|| unit.strip_prefix("dv"))
        .or_else(|| unit.strip_prefix("cq"))
        .or_else(|| unit.strip_prefix('v'))?;
    match base {
        "w" | "i" => Some(Axis::Width),
        "h" | "b" => Some(Axis::Height),
        "min" => Some(Axis::Min),
        "max" => Some(Axis::Max),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum Axis {
    Width,
    Height,
    Min,
    Max,
}

/// CSSOM "serialize a CSS component value" for a number as Chromium writes it
/// in a media query: six significant digits, `%g` style.
fn number(n: f64) -> String {
    if n == 0.0 {
        return "0".to_owned();
    }
    // `{:.5e}` rounds to six significant digits: "1.50000e2".
    let sci = format!("{n:.5e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let trim = |s: &str| -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            s.to_owned()
        }
    };
    if !(-4..6).contains(&exp) {
        let sign = if exp < 0 { '-' } else { '+' };
        return format!("{}e{sign}{:02}", trim(mantissa), exp.abs());
    }
    let decimals = usize::try_from(5 - exp).unwrap_or(0);
    trim(&format!("{n:.decimals$}"))
}

impl Query {
    fn serialize(&self, out: &mut String) {
        match self {
            Query::Condition(c) => c.serialize(out),
            Query::Typed {
                not,
                only,
                ty,
                condition,
            } => {
                // CSSOM §4.2.1: `all and` before a condition goes.
                if !not && !only && ty == "all" {
                    if let Some(c) = condition {
                        c.serialize(out);
                        return;
                    }
                }
                if *not {
                    out.push_str("not ");
                } else if *only {
                    out.push_str("only ");
                }
                // Writing to a String can't fail.
                let _ = cssparser::serialize_identifier(ty, out);
                if let Some(c) = condition {
                    out.push_str(" and ");
                    c.serialize(out);
                }
            }
        }
    }
}

impl Condition {
    fn serialize(&self, out: &mut String) {
        match self {
            Condition::Not(c) => {
                out.push_str("not ");
                c.serialize(out);
            }
            Condition::And(list) | Condition::Or(list) => {
                let word = if matches!(self, Condition::And(_)) {
                    " and "
                } else {
                    " or "
                };
                for (i, c) in list.iter().enumerate() {
                    if i > 0 {
                        out.push_str(word);
                    }
                    c.serialize(out);
                }
            }
            Condition::Parens(c) => {
                out.push('(');
                c.serialize(out);
                out.push(')');
            }
            Condition::Feature(f) => f.serialize(out),
            Condition::Enclosed(t) => out.push_str(t),
        }
    }
}

impl Feature {
    fn serialize(&self, out: &mut String) {
        out.push('(');
        match self {
            Feature::Boolean(k) => out.push_str(k.name),
            Feature::Plain {
                known,
                prefix,
                value,
            } => {
                if known.name == WEBKIT_DPR.name {
                    out.push_str("-webkit-");
                    out.push_str(match prefix {
                        Prefix::None => "",
                        Prefix::Min => "min-",
                        Prefix::Max => "max-",
                    });
                    out.push_str("device-pixel-ratio");
                } else {
                    out.push_str(match prefix {
                        Prefix::None => "",
                        Prefix::Min => "min-",
                        Prefix::Max => "max-",
                    });
                    out.push_str(known.name);
                }
                out.push_str(": ");
                value.serialize(out);
            }
            Feature::Range {
                known,
                before,
                after,
            } => {
                if let Some((v, op)) = before {
                    v.serialize(out);
                    out.push(' ');
                    out.push_str(op.text());
                    out.push(' ');
                }
                out.push_str(known.name);
                if let Some((op, v)) = after {
                    out.push(' ');
                    out.push_str(op.text());
                    out.push(' ');
                    v.serialize(out);
                }
            }
        }
        out.push(')');
    }
}

/// Evaluation, with MQ4 §3.2's three values: `None` is "unknown".
struct Eval<'a> {
    env: &'a Env,
    sized: bool,
}

/// What a feature is, for comparing.
enum Actual {
    Num(f64),
    /// width / height, maybe degenerate.
    Ratio(f64, f64),
    Word(&'static str),
}

impl Eval<'_> {
    fn query(&mut self, q: &Query) -> Option<bool> {
        match q {
            Query::Condition(c) => self.condition(c),
            Query::Typed {
                not, ty, condition, ..
            } => {
                // Scripts run under `screen`; print media is resolved by the
                // printer, separately.
                let typed = ty == "all" || ty == "screen";
                let r = match condition {
                    Some(c) if typed => self.condition(c),
                    _ => Some(typed),
                };
                if *not { r.map(|b| !b) } else { r }
            }
        }
    }

    fn condition(&mut self, c: &Condition) -> Option<bool> {
        match c {
            Condition::Not(c) => self.condition(c).map(|b| !b),
            Condition::And(list) => {
                let mut r = Some(true);
                for c in list {
                    match self.condition(c) {
                        Some(false) => return Some(false),
                        None => r = None,
                        Some(true) => {}
                    }
                }
                r
            }
            Condition::Or(list) => {
                let mut r = Some(false);
                for c in list {
                    match self.condition(c) {
                        Some(true) => return Some(true),
                        None => r = None,
                        Some(false) => {}
                    }
                }
                r
            }
            Condition::Parens(c) => self.condition(c),
            Condition::Feature(f) => Some(self.feature(f)),
            Condition::Enclosed(_) => None,
        }
    }

    fn actual(&mut self, k: &Known) -> Actual {
        let env = self.env;
        match k.answer {
            Answer::Width => {
                self.sized = true;
                Actual::Num(env.width)
            }
            Answer::Height => {
                self.sized = true;
                Actual::Num(env.height)
            }
            Answer::AspectRatio => {
                self.sized = true;
                Actual::Ratio(env.width, env.height)
            }
            Answer::Orientation => {
                self.sized = true;
                Actual::Word(if env.height >= env.width {
                    "portrait"
                } else {
                    "landscape"
                })
            }
            Answer::Resolution => Actual::Num(env.dpr),
            Answer::Int(i) => Actual::Num(f64::from(i)),
            Answer::Word(w) => Actual::Word(w),
        }
    }

    /// A value in the units of the feature's answer.
    fn amount(&mut self, v: &Value) -> Option<f64> {
        match v {
            Value::Length(px, _, None) => Some(*px),
            // Relative to the viewport, which is `width` and `height`.
            Value::Length(n, _, Some(axis)) => {
                self.sized = true;
                let env = self.env;
                let base = match axis {
                    Axis::Width => env.width,
                    Axis::Height => env.height,
                    Axis::Min => env.width.min(env.height),
                    Axis::Max => env.width.max(env.height),
                };
                Some(n * base / 100.0)
            }
            Value::Resolution(n, _) | Value::Number(n) => Some(*n),
            Value::Ratio(..) | Value::Keyword(_) => None,
        }
    }

    fn feature(&mut self, f: &Feature) -> bool {
        match f {
            Feature::Boolean(k) => match self.actual(k) {
                Actual::Num(n) => n != 0.0,
                Actual::Ratio(w, _) => w != 0.0,
                Actual::Word(w) => !matches!(w, "none" | "no-preference" | ""),
            },
            Feature::Plain {
                known,
                prefix,
                value,
            } => {
                let op = match prefix {
                    Prefix::None => Op::Eq,
                    Prefix::Min => Op::Ge,
                    Prefix::Max => Op::Le,
                };
                self.compare(known, op, value)
            }
            Feature::Range {
                known,
                before,
                after,
            } => {
                let mut ok = true;
                if let Some((v, op)) = before {
                    ok &= self.compare(known, op.flipped(), v);
                }
                if let Some((op, v)) = after {
                    ok &= self.compare(known, *op, v);
                }
                ok
            }
        }
    }

    /// `actual op value`.
    fn compare(&mut self, k: &Known, op: Op, v: &Value) -> bool {
        let actual = self.actual(k);
        match (actual, v) {
            (Actual::Word(w), Value::Keyword(want)) => op == Op::Eq && w == want,
            (Actual::Ratio(w, h), Value::Ratio(a, b)) => {
                // css-values-4 §5.? <ratio>: a degenerate ratio (a zero or an
                // infinite part) compares as nothing.
                let degenerate = |x: f64, y: f64| x == 0.0 || y == 0.0;
                if degenerate(w, h) || degenerate(*a, *b) {
                    return false;
                }
                // w/h op a/b, without dividing.
                cmp(w * b, op, a * h)
            }
            (Actual::Num(n), v) => match self.amount(v) {
                Some(x) => cmp(n, op, x),
                None => false,
            },
            _ => false,
        }
    }
}

fn cmp(a: f64, op: Op, b: f64) -> bool {
    match op {
        Op::Lt => a < b,
        Op::Le => a <= b,
        Op::Gt => a > b,
        Op::Ge => a >= b,
        Op::Eq => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chromium 153's viewport for the probes (1280 by 720 at 1x), so the
    /// answers below are its answers (scratch probe of P2-DOM-20a; the cases
    /// are listed for wpt/probes).
    const CHROME: Env = Env {
        width: 1280.0,
        height: 720.0,
        dpr: 1.0,
    };
    /// What a page sees today: `innerWidth` and `innerHeight` are 0.
    const PAGE: Env = Env {
        width: 0.0,
        height: 0.0,
        dpr: 1.0,
    };

    fn at(env: &Env, q: &str) -> (String, bool) {
        let e = match_media(q, env);
        (e.text, e.matches)
    }

    #[test]
    fn serializes_and_matches_as_chromium() {
        // (input, Chromium's `media`, Chromium's `matches` at 1280x720.)
        let cases: &[(&str, &str, bool)] = &[
            ("", "", true),
            (" ", "", true),
            ("all", "all", true),
            ("ALL", "all", true),
            ("screen", "screen", true),
            ("Screen", "screen", true),
            ("print", "print", false),
            ("speech", "speech", false),
            ("foo", "foo", false),
            ("not", "not all", false),
            ("only", "not all", false),
            ("and", "not all", false),
            ("or", "not all", false),
            ("layer", "not all", false),
            ("not screen", "not screen", false),
            ("NOT SCREEN", "not screen", false),
            ("only screen", "only screen", true),
            ("not print", "not print", true),
            ("not foo", "not foo", true),
            ("only foo", "only foo", false),
            ("screen, print", "screen, print", true),
            ("screen , print", "screen, print", true),
            ("screen,,print", "screen, not all, print", true),
            (",", "not all, not all", false),
            ("screen, foo bar", "screen, not all", true),
            ("screen and", "not all", false),
            ("screen and (color)", "screen and (color)", true),
            ("SCREEN AND (COLOR)", "screen and (color)", true),
            ("screen and(color)", "not all", false),
            (
                "screen and (color), print and (monochrome)",
                "screen and (color), print and (monochrome)",
                true,
            ),
            ("( color )", "(color)", true),
            (
                "(color) and (monochrome)",
                "(color) and (monochrome)",
                false,
            ),
            ("(color) or (monochrome)", "(color) or (monochrome)", true),
            ("(color) and (monochrome) or (hover)", "not all", false),
            ("not (color)", "not (color)", false),
            ("not (color) and (hover)", "not all", false),
            ("screen and not (color)", "screen and not (color)", false),
            (
                "screen and (not (color))",
                "screen and (not (color))",
                false,
            ),
            ("((color))", "((color))", true),
            ("(((color)))", "(((color)))", true),
            ("screen or (color)", "not all", false),
            ("not all and (color)", "not all and (color)", false),
            ("only (color)", "not all", false),
            ("(min-width: 100px)", "(min-width: 100px)", true),
            ("(MIN-WIDTH:100PX)", "(min-width: 100px)", true),
            ("(min-width: 0)", "(min-width: 0)", true),
            ("(min-width: 1)", "(min-width: 1)", false),
            ("(min-width: 10EM)", "(min-width: 10em)", true),
            ("(min-width: 1.5e2px)", "(min-width: 150px)", true),
            ("(min-width: +100px)", "(min-width: 100px)", true),
            ("(min-width: -100px)", "(min-width: -100px)", true),
            ("(min-width: 100.50px)", "(min-width: 100.5px)", true),
            ("(min-width: 1in)", "(min-width: 1in)", true),
            ("(min-width: 10vw)", "(min-width: 10vw)", true),
            ("(min-width: 50%)", "(min-width: 50%)", false),
            (
                "(min-width: 100px 200px)",
                "(min-width: 100px 200px)",
                false,
            ),
            ("(min-width)", "(min-width)", false),
            ("(width)", "(width)", true),
            ("(width: 0px)", "(width: 0px)", false),
            ("(width>=100px)", "(width >= 100px)", true),
            ("(100px <= width)", "(100px <= width)", true),
            ("(100px<width<2000px)", "(100px < width < 2000px)", true),
            ("(2000px > width > 100px)", "(2000px > width > 100px)", true),
            (
                "(100px < width > 2000px)",
                "(100px < width > 2000px)",
                false,
            ),
            ("(width = 100px)", "(width = 100px)", false),
            (
                "(width <= 100px <= 200px)",
                "(width <= 100px <= 200px)",
                false,
            ),
            ("(min-width >= 100px)", "(min-width >= 100px)", false),
            ("(width =< 100px)", "(width =< 100px)", false),
            ("(width < = 100px)", "(width < = 100px)", false),
            ("(orientation: landscape)", "(orientation: landscape)", true),
            ("(orientation: PORTRAIT)", "(orientation: portrait)", false),
            ("(orientation: foo)", "(orientation: foo)", false),
            ("(orientation)", "(orientation)", true),
            (
                "(min-orientation: portrait)",
                "(min-orientation: portrait)",
                false,
            ),
            ("(aspect-ratio: 16/9)", "(aspect-ratio: 16 / 9)", true),
            ("(aspect-ratio: 16)", "(aspect-ratio: 16 / 1)", false),
            ("(aspect-ratio: 1.5)", "(aspect-ratio: 1.5 / 1)", false),
            ("(aspect-ratio: 16/9.5)", "(aspect-ratio: 16 / 9.5)", false),
            ("(min-aspect-ratio: 1/1)", "(min-aspect-ratio: 1 / 1)", true),
            (
                "(max-aspect-ratio: 1/1)",
                "(max-aspect-ratio: 1 / 1)",
                false,
            ),
            ("(aspect-ratio > 1/1)", "(aspect-ratio > 1 / 1)", true),
            ("(aspect-ratio)", "(aspect-ratio)", true),
            ("(resolution: 1dppx)", "(resolution: 1dppx)", true),
            ("(resolution: 96dpi)", "(resolution: 96dpi)", true),
            ("(resolution: 2x)", "(resolution: 2x)", false),
            ("(max-resolution: 1dppx)", "(max-resolution: 1dppx)", true),
            ("(resolution > 1dppx)", "(resolution > 1dppx)", false),
            ("(resolution: 1)", "(resolution: 1)", false),
            ("(resolution)", "(resolution)", true),
            (
                "(-webkit-device-pixel-ratio: 1)",
                "(-webkit-device-pixel-ratio: 1)",
                true,
            ),
            (
                "(-webkit-min-device-pixel-ratio: 1)",
                "(-webkit-min-device-pixel-ratio: 1)",
                true,
            ),
            (
                "(-webkit-max-device-pixel-ratio: 2)",
                "(-webkit-max-device-pixel-ratio: 2)",
                true,
            ),
            ("(device-pixel-ratio: 1)", "(device-pixel-ratio: 1)", false),
            (
                "(prefers-color-scheme: light)",
                "(prefers-color-scheme: light)",
                true,
            ),
            (
                "(prefers-color-scheme: dark)",
                "(prefers-color-scheme: dark)",
                false,
            ),
            ("(prefers-color-scheme)", "(prefers-color-scheme)", true),
            (
                "(prefers-reduced-motion: no-preference)",
                "(prefers-reduced-motion: no-preference)",
                true,
            ),
            (
                "(prefers-reduced-motion)",
                "(prefers-reduced-motion)",
                false,
            ),
            ("(color: 8)", "(color: 8)", true),
            ("(min-color: 1)", "(min-color: 1)", true),
            ("(color > 4)", "(color > 4)", true),
            ("(color: 1.5)", "(color: 1.5)", false),
            ("(color: -1)", "(color: -1)", false),
            ("(monochrome)", "(monochrome)", false),
            ("(monochrome: 0)", "(monochrome: 0)", true),
            ("(max-monochrome: 0)", "(max-monochrome: 0)", true),
            ("(color-index)", "(color-index)", false),
            ("(grid)", "(grid)", false),
            ("(grid: 0)", "(grid: 0)", true),
            ("(scan: progressive)", "(scan: progressive)", false),
            ("(update: fast)", "(update: fast)", true),
            ("(overflow-block: scroll)", "(overflow-block: scroll)", true),
            ("(color-gamut: srgb)", "(color-gamut: srgb)", true),
            (
                "(dynamic-range: standard)",
                "(dynamic-range: standard)",
                true,
            ),
            ("(forced-colors: none)", "(forced-colors: none)", true),
            ("(forced-colors)", "(forced-colors)", false),
            (
                "(prefers-contrast: no-preference)",
                "(prefers-contrast: no-preference)",
                true,
            ),
            ("(prefers-contrast)", "(prefers-contrast)", false),
            ("(scripting: enabled)", "(scripting: enabled)", true),
            ("(scripting)", "(scripting)", true),
            ("(display-mode: browser)", "(display-mode: browser)", true),
            (
                "(min-device-width: 100px)",
                "(min-device-width: 100px)",
                true,
            ),
            (
                "(device-aspect-ratio: 16/9)",
                "(device-aspect-ratio: 16 / 9)",
                true,
            ),
            ("(foo)", "(foo)", false),
            ("(foo: bar)", "(foo: bar)", false),
            ("screen and (foo)", "screen and (foo)", false),
            ("(color) and (foo)", "(color) and (foo)", false),
            ("(color) or (foo)", "(color) or (foo)", true),
            ("not (foo)", "not (foo)", false),
            ("screen, (foo)", "screen, (foo)", true),
            ("(color:)", "(color:)", false),
            ("(: 1)", "(: 1)", false),
            ("(min-width: 100px) and", "not all", false),
            ("(min-width: 100px) (max-width: 200px)", "not all", false),
            ("foo(bar)", "foo(bar)", false),
            ("screen and foo(bar)", "screen and foo(bar)", false),
            ("[color]", "not all", false),
            ("{color}", "not all", false),
            ("screen;", "not all", false),
            ("screen; print", "not all", false),
            ("@media screen", "not all", false),
            (
                "screen /* c */ and /* d */ (color)",
                "screen and (color)",
                true,
            ),
            ("screen\\,print", "screen\\,print", false),
            ("scr\\65 en", "screen", true),
            (
                "(width < 100px) or (width > 100px)",
                "(width < 100px) or (width > 100px)",
                true,
            ),
            (
                "only screen and (min-width: 768px) and (max-width: 1023px)",
                "only screen and (min-width: 768px) and (max-width: 1023px)",
                false,
            ),
            ("(min-width: 0\\9)", "(min-width: 0\\9)", false),
            (
                "screen and (min-width:0\\0)",
                "screen and (min-width:0\\0)",
                false,
            ),
            (
                "all and (-ms-high-contrast: none)",
                "(-ms-high-contrast: none)",
                false,
            ),
            ("(width: 1e400px)", "(width: 3.40282e+38px)", false),
            ("(width: 0.000001px)", "(width: 1e-06px)", false),
            (
                "(min-width: 100px)  ,  (max-width: 200px)",
                "(min-width: 100px), (max-width: 200px)",
                true,
            ),
            ("screen\t", "screen", true),
            ("(width: 1Q)", "(width: 1q)", false),
            ("(width: 1cap)", "(width: 1cap)", false),
        ];
        for (q, text, matches) in cases {
            assert_eq!(at(&CHROME, q), ((*text).to_owned(), *matches), "{q:?}");
        }
    }

    #[test]
    fn what_a_page_sees_today() {
        // A window of 0 by 0: no width, so `(width)` is false, `max-*`
        // matches, and the degenerate ratio 0/0 compares as nothing.
        for (q, m) in [
            ("(width)", false),
            ("(min-width: 1px)", false),
            ("(max-width: 767px)", true),
            ("(orientation: portrait)", true),
            ("(aspect-ratio)", false),
            ("(min-aspect-ratio: 1/1)", false),
            ("(max-aspect-ratio: 1/1)", false),
            ("(hover: hover)", false),
            ("(hover: none)", true),
            ("(pointer: none)", true),
            ("(any-pointer: fine)", false),
            ("print", false),
            ("screen", true),
            ("(min-width: 10vw)", true),
        ] {
            assert_eq!(at(&PAGE, q).1, m, "{q:?}");
        }
        assert!(match_media("(min-width: 10px)", &PAGE).sized);
        assert!(!match_media("(color)", &PAGE).sized);
        assert!(
            match_media("(min-resolution: 10vw)", &PAGE)
                .text
                .contains("10vw")
        );
    }

    #[test]
    fn hostile_queries_are_bounded() {
        let big = "a".repeat(MAX_QUERY + 1);
        assert_eq!(at(&PAGE, &big), ("not all".to_owned(), false));
        // Nesting past the cap is `not all`, at any depth.
        for depth in [MAX_NESTING, MAX_NESTING + 1, 10_000] {
            let q = format!("{}color{}", "(".repeat(depth), ")".repeat(depth));
            let e = match_media(&q, &PAGE);
            if depth <= MAX_NESTING {
                assert!(e.matches, "{depth}");
            } else {
                assert_eq!(e.text, "not all", "{depth}");
            }
            let q = format!("foo({}x{})", "(".repeat(depth), ")".repeat(depth));
            let _ = match_media(&q, &PAGE);
        }
        // A list at the cap: linear.
        let q = "(min-width: 1px), ".repeat(MAX_QUERY / 20);
        let e = match_media(&q, &PAGE);
        assert!(e.text.ends_with("not all"));
        for q in [
            "(", ")", "(\"", "url(", "(\\", "\u{0}", "((((", "))))", "(a:b", "(width <", "(1px <",
        ] {
            let _ = match_media(q, &PAGE);
        }
    }

    #[test]
    fn numbers_as_chromium_writes_them() {
        for (n, s) in [
            (100.0, "100"),
            (100.5, "100.5"),
            (150.0, "150"),
            (0.000001, "1e-06"),
            (f64::from(f32::MAX), "3.40282e+38"),
            (1_000_000.0, "1e+06"),
            (123_456.0, "123456"),
            (0.1, "0.1"),
            (-100.0, "-100"),
            (0.0001, "0.0001"),
        ] {
            assert_eq!(number(n), s, "{n}");
        }
    }
}
