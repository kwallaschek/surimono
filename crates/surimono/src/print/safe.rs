//! Printer-safe output: the page rewritten into forms stock fulgur prints the
//! way Chrome prints the original.
//!
//! fulgur 0.40 and 0.41 (blitz-dom 0.2.4) print some ordinary pages wrong
//! although their DOM is right: a block-level `<svg>` loses its `width` and
//! `height`, a border on one side prints in the text colour, and a `<pre>`
//! ending in a newline is a line too tall. Each has an equivalent form that
//! prints right. This pass writes those forms into the page's own output, so
//! they reach every fulgur user with no change to the printer.
//!
//! **The rule that decides what is rewritten:** Chrome must render the
//! rewritten page exactly like the original, under any author style sheet and
//! under paged media. A rewrite that could change what Chrome shows for some
//! style sheet isn't made. Where that can't be shown from the page itself, the
//! page is left as it is: the pass has no style engine (a stylo run can't be
//! stopped, and nothing here may run unbounded), so every condition below is
//! read off the printer's tree and the CSS's tokens ([`super::rules`]), and
//! errs towards not rewriting.
//!
//! # The three rewrites
//!
//! **Size of a block `<svg>`** (blitz-dom ignores `width` and `height` on an
//! `<svg>` that isn't inline). Chrome maps both attributes of an outermost
//! `<svg>` to presentational hints: declarations just below the author origin.
//! The pass adds the same declarations as the lowest thing in the author
//! origin: a rule per attribute value, `svg[width="640"] { width: 640px }`, in
//! an anonymous cascade layer of a style sheet placed before every author
//! sheet. In Chrome it only repeats what the hint beneath it says, so every
//! author declaration, layered or not, `!important` or not, still wins, and
//! `revert` and `revert-layer` land on the same value as before. An inline
//! `style` would outrank author rules the attribute loses to, which is why
//! this isn't one. The selector is namespaced (a MathML element can be named
//! `svg` too) and leaves out an `<svg>` inside another, whose attributes are no
//! hints. Lengths are SVG's: a bare number is pixels.
//!
//! **Colour of a border on one side** (fulgur paints all four sides in
//! `border-top-color`). For each rule with a `border-bottom`, `-left` or
//! `-right` that paints, the pass adds `border-top-color` with that side's
//! colour, under the same selector, in the same lowest layer. It can only take
//! effect where no author declaration sets the top border's colour, and it is
//! invisible in Chrome as long as no top border paints there and nothing
//! reads the colour another way. That holds when: every border style on the
//! page comes from a shorthand that sets the colour too (no `border-style`
//! longhand anywhere, so a top border that paints has an author colour that
//! outranks the layer); the selector can't match an element the user agent
//! gives a border (form controls, `<hr>`, `<fieldset>`, anything with
//! `popover`); no `border`, `rules` or `frame` attribute gives a table one; no
//! border declaration says `inherit`, and nothing says `revert-layer`. For the
//! colour to be the one that prints, all the one-sided borders of the page
//! have to be readable this way (plain rules, literal colours, no
//! `!important`), so that the copies cascade among themselves as the
//! originals do; otherwise none is copied.
//!
//! **Newline ending a `<pre>`** (parley gives a layout ending in a newline one
//! more line, and blitz-dom keeps it). Chrome ends the last line at that
//! newline and starts no other, so the newline goes, when: it follows a
//! visible character in the same text node (after an empty line it *is* the
//! line: `<pre>one\n\n</pre>` is two lines in Chrome); the text is the last
//! thing in the `<pre>`, through plain inline elements only; the `<pre>` is a
//! block container whatever the page's CSS says (no rule that could match it
//! declares another `display`, or `all`); no `::after` with `content` could
//! put something behind the newline; and nothing says `preserve-spaces`.
//! No selector sees a text node, and the node isn't emptied, so nothing else
//! can tell.
//!
//! # What the added style sheet needs
//!
//! The first two rewrites travel in one new `<style>` in `<head>`, after the
//! leading `<meta>`, `<title>` and `<base>` elements, where it precedes every
//! author sheet without moving a `<meta charset>`. A new element is something
//! selectors can see, so it is added only when the page's CSS can't:
//!
//! - all of the page's CSS is in the page (no linked style sheet, no
//!   `@import`); load them with a [`ResourceLoader`](crate::ResourceLoader)
//!   and they are;
//! - no selector uses `:has()`, the only way a change inside `<head>` reaches
//!   an element that is rendered;
//! - no rule that could match `<head>` declares a `display` other than `none`,
//!   or `all`, so what's in it stays unrendered;
//! - `<head>` already has an element, so `head:empty` says what it said;
//! - the page is ASCII or declares its encoding before the new element, so a
//!   browser reading it from a file decodes it as before.
//!
//! And nothing is rewritten at all in a page that keeps its scripts
//! ([`Options::keep_scripts`](crate::Options::keep_scripts)), since a script
//! can see everything.

use cssparser::{Parser, ParserInput, Token};
use html5ever::ns;

use super::rules::{Origin, Rule, Rules, Subject};
use super::tree::{Data, Element, Tree};
use super::{Error, Gate};
use crate::prepass::is_island_type;

/// Most rules the added style sheet may hold.
const MAX_RULES: usize = 256;
/// Most bytes of it.
const MAX_SHEET_BYTES: usize = 32 * 1024;

/// HTML elements Chrome's own style sheet gives no border, in any state.
const BORDERLESS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "article",
    "aside",
    "b",
    "big",
    "blockquote",
    "body",
    "br",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "html",
    "i",
    "ins",
    "kbd",
    "label",
    "li",
    "main",
    "mark",
    "nav",
    "ol",
    "p",
    "pre",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "u",
    "ul",
    "var",
    "wbr",
];

/// Attributes that give an element, or a table's cells, a border.
const BORDER_ATTRIBUTES: &[&str] = &[
    "border",
    "rules",
    "frame",
    "bordercolor",
    "frameborder",
    "noshade",
];

/// Inline HTML elements a `<pre>`'s last text may be in: they add nothing
/// after their content (`<q>` adds a quote) and hold only what's in the tree.
const PLAIN_INLINE: &[&str] = &[
    "a", "abbr", "b", "cite", "code", "data", "del", "dfn", "em", "font", "i", "ins", "kbd",
    "mark", "s", "samp", "small", "span", "strong", "sub", "sup", "time", "tt", "u", "var",
];

/// SMIL elements: an animated `width` isn't the attribute's.
const SMIL: &[&str] = &[
    "animate",
    "set",
    "animatemotion",
    "animatetransform",
    "animatecolor",
    "discard",
];

/// `display` values that make a `<pre>` a block container, or nothing.
const BLOCK_DISPLAYS: &[&str] = &[
    "block",
    "flow-root",
    "inline-block",
    "list-item",
    "flex",
    "grid",
    "inline-flex",
    "inline-grid",
    "none",
];

/// Units an SVG length and a CSS length share, with the same meaning.
const ABSOLUTE_UNITS: &[&str] = &["px", "in", "cm", "mm", "pt", "pc"];

/// What one walk over the tree finds.
#[derive(Default)]
struct Facts {
    head: Option<usize>,
    /// A script that isn't an island marker is still in the page.
    scripts: bool,
    /// A `<link>` to a style sheet: CSS that isn't in the page.
    links: bool,
    smil: bool,
    border_attributes: bool,
    /// `<style>` elements, in any namespace and in templates: a printer reads
    /// them all, and Chrome some.
    styles: Vec<usize>,
    /// Elements with a `style` attribute.
    styled: Vec<usize>,
    /// Elements that may have a border of the user agent's.
    bordered: Vec<usize>,
    /// `width` and `height` values of outermost `<svg>` elements, each once,
    /// in document order.
    sizes: Vec<(&'static str, String)>,
    /// HTML `<pre>` elements outside templates.
    pres: Vec<usize>,
}

fn is_svg(e: &Element) -> bool {
    e.name.ns == ns!(svg)
}

fn facts(tree: &Tree, gate: &Gate<'_>) -> Result<Facts, Error> {
    let mut facts = Facts::default();
    let html = tree.nodes[0]
        .children
        .iter()
        .copied()
        .find(|&c| tree.element(c).is_some_and(|e| e.is_html("html")));
    facts.head = html.and_then(|html| {
        tree.nodes[html]
            .children
            .iter()
            .copied()
            .find(|&c| tree.element(c).is_some_and(|e| e.is_html("head")))
    });
    let mut stack = vec![(0, false)];
    while let Some((id, inert)) = stack.pop() {
        gate.check()?;
        let node = &tree.nodes[id];
        let mut inside = inert;
        if let Some(e) = tree.element(id) {
            let local = e.local();
            inside |= e.is_html("template");
            if local == "script" && !inert {
                let island = e.is_html("script") && e.attr("type").is_some_and(is_island_type);
                facts.scripts |= !island;
            }
            if local.eq_ignore_ascii_case("link")
                && e.attr("rel")
                    .is_some_and(|r| r.to_ascii_lowercase().contains("stylesheet"))
            {
                facts.links = true;
            }
            facts.smil |= SMIL.iter().any(|s| local.eq_ignore_ascii_case(s));
            for a in &e.attrs {
                let name = a.name.local.as_ref();
                facts.border_attributes |= BORDER_ATTRIBUTES.contains(&name);
                if name == "style" {
                    facts.styled.push(id);
                }
            }
            if local == "style" {
                facts.styles.push(id);
            }
            let html_element = e.name.ns == ns!(html);
            let borderless = html_element
                && e.attr("popover").is_none()
                && (BORDERLESS.contains(&local) || local.contains('-'));
            if !borderless {
                facts.bordered.push(id);
            }
            if html_element && local == "pre" && !inert {
                facts.pres.push(id);
            }
            let outermost = is_svg(e)
                && local == "svg"
                && !node
                    .parent
                    .and_then(|p| tree.element(p))
                    .is_some_and(is_svg);
            if outermost {
                for name in ["width", "height"] {
                    let value = e
                        .attrs
                        .iter()
                        .find(|a| a.name.ns == ns!() && a.name.local.as_ref() == name);
                    if let Some(a) = value
                        && svg_length(&a.value).is_some()
                        && !facts
                            .sizes
                            .iter()
                            .any(|(n, v)| *n == name && *v == *a.value)
                    {
                        facts.sizes.push((name, a.value.to_string()));
                    }
                }
            }
        }
        stack.extend(node.children.iter().rev().map(|&c| (c, inside)));
    }
    Ok(facts)
}

/// The CSS length that says what the SVG length `value` says, for the plain
/// forms: digits, a fraction, and an absolute unit or none (user units, which
/// are pixels). Anything else is left to the printer.
fn svg_length(value: &str) -> Option<String> {
    let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
    let whole = digits(value);
    if !(1..=7).contains(&whole) {
        return None;
    }
    let mut end = whole;
    if value[end..].starts_with('.') {
        let fraction = digits(&value[end + 1..]);
        if !(1..=4).contains(&fraction) {
            return None;
        }
        end += 1 + fraction;
    }
    let unit = &value[end..];
    if unit.is_empty() {
        return Some(format!("{value}px"));
    }
    ABSOLUTE_UNITS.contains(&unit).then(|| value.to_owned())
}

/// A value's tokens, as far as a border or a `display` needs them.
#[derive(Debug, PartialEq, Eq)]
enum Word {
    /// In lower case.
    Ident(String),
    /// A hash, or `rgb()` or `hsl()` of plain numbers, as written.
    Colour(String),
    /// A number or a length.
    Size,
    /// `!`, as in `!important`.
    Bang,
    Other,
}

/// The words of a declaration's value.
fn words(value: &str) -> Vec<Word> {
    let mut input = ParserInput::new(value);
    let mut p = Parser::new(&mut input);
    let mut all = Vec::new();
    loop {
        let start = p.position();
        let Ok(token) = p.next() else {
            break;
        };
        let word = match token.clone() {
            Token::Ident(name) => Word::Ident(name.to_ascii_lowercase()),
            Token::Hash(_) | Token::IDHash(_) => {
                Word::Colour(p.slice_from(start).trim().to_owned())
            }
            Token::Number { .. } | Token::Dimension { .. } => Word::Size,
            Token::Delim('!') => Word::Bang,
            Token::Function(name)
                if ["rgb", "rgba", "hsl", "hsla"]
                    .iter()
                    .any(|f| name.eq_ignore_ascii_case(f)) =>
            {
                let plain: Result<bool, cssparser::ParseError<'_, ()>> =
                    p.parse_nested_block(|p| {
                        let mut plain = true;
                        while let Ok(token) = p.next() {
                            plain &= matches!(
                                token,
                                Token::Number { .. }
                                    | Token::Percentage { .. }
                                    | Token::Comma
                                    | Token::Delim('/')
                            );
                        }
                        Ok(plain)
                    });
                let text = p.slice_from(start).trim();
                if plain == Ok(true) && text.is_ascii() && !text.contains(['\\', '<', '&']) {
                    Word::Colour(text.to_owned())
                } else {
                    Word::Other
                }
            }
            _ => Word::Other,
        };
        all.push(word);
        if all.len() > 8 {
            return vec![Word::Other];
        }
    }
    all
}

/// Whether `value` is the one keyword `none`, important or not.
fn is_none(value: &str) -> bool {
    one_ident(value).as_deref() == Some("none")
}

/// The keyword `value` is, if it is one, `!important` aside.
fn one_ident(value: &str) -> Option<String> {
    let mut all = words(value);
    if all.len() == 3 && all[1] == Word::Bang && all[2] == Word::Ident("important".into()) {
        all.truncate(1);
    }
    match (all.pop(), all.is_empty()) {
        (Some(Word::Ident(name)), true) => Some(name),
        _ => None,
    }
}

/// Where the new `<style>` goes, if the page's CSS can't see one: `<head>` and
/// the index among its children. See the module docs.
fn carrier(tree: &Tree, facts: &Facts, rules: &Rules, ascii: bool) -> Option<(usize, usize)> {
    let head = facts.head?;
    let element = tree.element(head)?;
    if rules.flags.has {
        return None;
    }
    let children = &tree.nodes[head].children;
    if !children.iter().any(|&c| tree.element(c).is_some()) {
        return None;
    }
    let mut declared = false;
    let mut at = children.len();
    for (i, &child) in children.iter().enumerate() {
        let Some(e) = tree.element(child) else {
            continue;
        };
        if e.is_html("meta") {
            declared |= e.attr("charset").is_some() || e.attr("http-equiv").is_some();
        } else if !e.is_html("title") && !e.is_html("base") {
            at = i;
            break;
        }
    }
    if !ascii && !declared {
        return None;
    }
    let shown = rules.rules.iter().any(|rule| {
        rule.may_match(head, element)
            && rule
                .decls
                .iter()
                .any(|d| d.name == "all" || (d.name == "display" && !is_none(rules.value(rule, d))))
    });
    (!shown).then_some((head, at))
}

/// The rules that size outermost `<svg>` elements as their attributes do.
fn svg_sizes(facts: &Facts) -> Vec<String> {
    if facts.smil {
        return Vec::new();
    }
    facts
        .sizes
        .iter()
        .filter_map(|(name, value)| {
            let length = svg_length(value)?;
            Some(format!(
                "s|svg[{name}=\"{value}\"]:not(s|svg *){{{name}:{length}}}"
            ))
        })
        .collect()
}

/// What a declaration does to the colour of the one side it is for.
#[derive(Debug, PartialEq, Eq)]
enum Side {
    /// The side doesn't paint by it, or keeps its colour.
    Unpainted,
    /// It paints in this colour.
    Colour(String),
    /// It can't be told here.
    Unknown,
}

fn is_border_style(name: &str) -> Option<bool> {
    match name {
        "none" | "hidden" => Some(false),
        "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" => {
            Some(true)
        }
        _ => None,
    }
}

fn is_wide_keyword(name: &str) -> bool {
    matches!(
        name,
        "initial" | "inherit" | "unset" | "revert" | "revert-layer"
    )
}

/// A colour keyword as far as can be told here: an identifier that is nothing
/// else. If it is no colour either, Chrome drops the declaration and the copy.
fn colour_name(name: &str) -> bool {
    !is_wide_keyword(name)
        && is_border_style(name).is_none()
        && !matches!(name, "thin" | "medium" | "thick" | "important")
        && name.bytes().all(|b| b.is_ascii_alphabetic())
}

/// Reads `border-bottom: 1px solid #ccc` and the like.
fn side_shorthand(value: &str) -> Side {
    let all = words(value);
    if let [Word::Ident(name)] = all.as_slice()
        && matches!(name.as_str(), "initial" | "unset" | "revert")
    {
        return Side::Unpainted;
    }
    let (mut paints, mut colour) = (false, None);
    for word in all {
        match word {
            Word::Size => {}
            Word::Ident(name) if matches!(name.as_str(), "thin" | "medium" | "thick") => {}
            Word::Ident(name) => match is_border_style(&name) {
                Some(style) => paints = style,
                None if colour.is_none() && colour_name(&name) => colour = Some(name),
                None => return Side::Unknown,
            },
            Word::Colour(text) if colour.is_none() => colour = Some(text),
            _ => return Side::Unknown,
        }
    }
    if !paints {
        return Side::Unpainted;
    }
    Side::Colour(colour.unwrap_or_else(|| "currentcolor".to_owned()))
}

/// Reads `border-bottom-color: #ccc`.
fn side_colour(value: &str) -> Side {
    match words(value).as_slice() {
        [Word::Colour(text)] => Side::Colour(text.clone()),
        [Word::Ident(name)] if matches!(name.as_str(), "initial" | "unset" | "revert") => {
            Side::Colour("currentcolor".to_owned())
        }
        [Word::Ident(name)] if colour_name(name) => Side::Colour(name.clone()),
        _ => Side::Unknown,
    }
}

/// Whether a selector can be written into the added sheet as it is: nothing
/// that means something else there (a namespace prefix, a nesting selector),
/// or that a printer or a parser would read differently from Chrome.
fn copyable(selector: &str) -> bool {
    !selector.is_empty()
        && selector
            .bytes()
            .all(|b| b == b' ' || b.is_ascii_graphic() && !b"&<\\|@{};".contains(&b))
}

/// `selector` with each run of white space as one space, which is what a line
/// break in a selector list means. `None` if it holds a string, where white
/// space is itself.
fn on_one_line(selector: &str) -> Option<String> {
    if selector.contains(['"', '\'']) && selector.contains(['\n', '\t', '\r', '\u{c}']) {
        return None;
    }
    Some(
        selector
            .split_ascii_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Whether the `<style>` at `node` applies as written, in print as on screen.
fn applies_everywhere(tree: &Tree, node: usize) -> bool {
    tree.element(node).is_some_and(|e| {
        e.attr("media").is_none()
            && e.attr("title").is_none()
            && e.attr("type")
                .is_none_or(|t| t.is_empty() || t.eq_ignore_ascii_case("text/css"))
    })
}

/// The rules that give a one-sided border's colour to `border-top-color`, or
/// none if the page's borders can't all be read; see the module docs.
fn border_colours(
    tree: &Tree,
    facts: &Facts,
    rules: &Rules,
    gate: &Gate<'_>,
) -> Result<Vec<String>, Error> {
    const SIDES: [&str; 3] = ["border-bottom", "border-left", "border-right"];
    const TOP: [&str; 4] = ["border", "border-top", "border-color", "border-top-color"];
    let none = Ok(Vec::new());
    if rules.flags.revert_layer || facts.border_attributes {
        return none;
    }
    let mut copies = Vec::new();
    let mut bytes = 0;
    for rule in &rules.rules {
        gate.check()?;
        let mut colours = Vec::new();
        for decl in &rule.decls {
            let name = decl.name.as_str();
            if name == "all" {
                return none;
            }
            if !name.starts_with("border") {
                continue;
            }
            let value = rules.value(rule, decl);
            let logical = name.starts_with("border-block") || name.starts_with("border-inline");
            if name.ends_with("-style")
                || logical
                || words(value).contains(&Word::Ident("inherit".to_owned()))
            {
                return none;
            }
            let side = if SIDES.contains(&name) {
                side_shorthand(value)
            } else if SIDES
                .iter()
                .any(|s| name.strip_prefix(s).is_some_and(|rest| rest == "-color"))
            {
                side_colour(value)
            } else {
                continue;
            };
            match side {
                Side::Unpainted => {}
                Side::Unknown => return none,
                Side::Colour(colour) => colours.push(colour),
            }
        }
        if colours.is_empty() {
            continue;
        }
        if !mirrors(tree, facts, rules, rule) {
            return none;
        }
        if rule.decls.iter().any(|d| TOP.contains(&d.name.as_str())) {
            continue;
        }
        let Some(selector) = on_one_line(rules.selector(rule)) else {
            return none;
        };
        for colour in colours {
            let copy = format!("{selector}{{border-top-color:{colour}}}");
            bytes += copy.len();
            copies.push(copy);
        }
        if copies.len() > MAX_RULES || bytes > MAX_SHEET_BYTES {
            return none;
        }
    }
    Ok(copies)
}

/// Whether `rule`, which paints one side, can be copied: a plain rule of a
/// sheet that always applies, whose selector means the same in the added
/// sheet and can't match a pseudo-element or an element with a border of the
/// user agent's.
fn mirrors(tree: &Tree, facts: &Facts, rules: &Rules, rule: &Rule) -> bool {
    let Origin::Sheet(node) = rule.origin else {
        return false;
    };
    rule.plain
        && applies_everywhere(tree, node)
        && on_one_line(rules.selector(rule)).is_some_and(|s| copyable(&s))
        && !rule.subjects.is_empty()
        && rule.subjects.iter().all(|s| !s.any && !s.pseudo_element)
        && !facts.bordered.iter().any(|&id| {
            tree.element(id)
                .is_some_and(|e| rule.subjects.iter().any(|s| s.may_match(e)))
        })
}

/// Whether a line that ends in `c` has content, whatever the CSS: not white
/// space, not a control, and nothing a font leaves out (default ignorables).
fn visible(c: char) -> bool {
    let ignorable = matches!(
        c,
        '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{115f}'..='\u{1160}' | '\u{17b4}'..='\u{17b5}'
            | '\u{180b}'..='\u{180f}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}' | '\u{3164}' | '\u{fe00}'..='\u{fe0f}' | '\u{feff}'
            | '\u{ffa0}' | '\u{fff0}'..='\u{fff8}' | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}' | '\u{e0000}'..='\u{e0fff}'
    );
    !c.is_whitespace() && !c.is_control() && !ignorable
}

/// The text nodes whose last newline Chrome doesn't render: the end of a
/// `<pre>`; see the module docs.
fn trailing_newlines(tree: &Tree, facts: &Facts, rules: &Rules) -> Vec<usize> {
    if rules.flags.preserve_spaces {
        return Vec::new();
    }
    let mut found = Vec::new();
    'pre: for &pre in &facts.pres {
        // The elements the text is in, from the `<pre>` down.
        let mut path = vec![pre];
        let mut at = pre;
        let text = loop {
            let last = tree.nodes[at]
                .children
                .iter()
                .rev()
                .copied()
                .find(|&c| !matches!(tree.nodes[c].data, Data::Comment));
            let Some(last) = last else {
                continue 'pre;
            };
            match &tree.nodes[last].data {
                Data::Text(_) => break last,
                Data::Element(e) if e.name.ns == ns!(html) && PLAIN_INLINE.contains(&e.local()) => {
                    path.push(last);
                    at = last;
                }
                _ => continue 'pre,
            }
        };
        let Data::Text(content) = &tree.nodes[text].data else {
            continue;
        };
        let mut end = content.chars().rev();
        if end.next() != Some('\n') || !end.next().is_some_and(visible) {
            continue;
        }
        let Some(element) = tree.element(pre) else {
            continue;
        };
        for rule in &rules.rules {
            let inline = rule.may_match(pre, element)
                && rule.decls.iter().any(|d| {
                    d.name == "all"
                        || (d.name == "display"
                            && !one_ident(rules.value(rule, d))
                                .is_some_and(|v| BLOCK_DISPLAYS.contains(&v.as_str())))
                });
            let behind = |s: &Subject| {
                s.after
                    && path
                        .iter()
                        .any(|&id| tree.element(id).is_some_and(|e| s.may_match(e)))
            };
            let after = matches!(rule.origin, Origin::Sheet(_))
                && rule
                    .decls
                    .iter()
                    .any(|d| d.name == "content" || d.name == "all")
                && rule.subjects.iter().any(behind);
            if inline || after {
                continue 'pre;
            }
        }
        found.push(text);
    }
    found
}

/// Rewrites `tree`, the printer's parse of a page that has passed every
/// screen, into its printer-safe form, and says whether anything changed.
/// `ascii` says whether the page's text is all ASCII. See the module docs.
pub(super) fn rewrite(tree: &mut Tree, ascii: bool, gate: &Gate<'_>) -> Result<bool, Error> {
    let facts = facts(tree, gate)?;
    if facts.scripts {
        return Ok(false);
    }
    let texts: Vec<(usize, String)> = facts
        .styles
        .iter()
        .map(|&id| (id, tree.text_of(id)))
        .collect();
    let borders = texts.iter().any(|(_, css)| {
        let css = css.to_ascii_lowercase();
        ["border-bottom", "border-left", "border-right"]
            .iter()
            .any(|side| css.contains(side))
    });
    if facts.sizes.is_empty() && facts.pres.is_empty() && !borders {
        return Ok(false);
    }
    gate.check()?;
    let mut rules = Rules::default();
    for (id, css) in texts {
        rules.read_sheet(id, css, gate)?;
    }
    for &id in &facts.styled {
        let Some(e) = tree.element(id) else {
            continue;
        };
        for a in e.attrs.iter().filter(|a| a.name.local.as_ref() == "style") {
            rules.read_attribute(id, a.value.to_string(), gate)?;
        }
    }
    // CSS that isn't here, or wasn't read to its end, could say anything.
    if rules.unread || rules.flags.import || facts.links {
        return Ok(false);
    }
    let mut changed = false;
    for text in trailing_newlines(tree, &facts, &rules) {
        if let Data::Text(content) = &mut tree.nodes[text].data {
            content.pop();
            changed = true;
        }
    }
    gate.check()?;
    if let Some((head, at)) = carrier(tree, &facts, &rules, ascii) {
        let sizes = svg_sizes(&facts);
        let colours = if borders {
            border_colours(tree, &facts, &rules, gate)?
        } else {
            Vec::new()
        };
        let count = sizes.len() + colours.len();
        let bytes: usize = sizes.iter().chain(&colours).map(String::len).sum();
        if count > 0 && count <= MAX_RULES && bytes <= MAX_SHEET_BYTES {
            let mut css = String::with_capacity(bytes + 64);
            if !sizes.is_empty() {
                css.push_str("@namespace s url(http://www.w3.org/2000/svg);");
            }
            css.push_str("@layer{");
            for rule in sizes.iter().chain(&colours) {
                css.push_str(rule);
            }
            css.push('}');
            tree.insert_style(head, at, css);
            changed = true;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use super::super::{Finish, finish_page};
    use super::*;

    const WIDE: usize = usize::MAX / 8;

    fn finish(html: &str, printer_safe: bool, max_bytes: usize) -> Result<String, Error> {
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::from_secs(600));
        let finish = Finish {
            keep_scripts: false,
            max_bytes,
            printer_safe,
        };
        finish_page(html, finish, None, &gate)
    }

    fn safe(html: &str) -> String {
        finish(html, true, WIDE).unwrap_or_else(|e| panic!("{e}"))
    }

    fn plain(html: &str) -> String {
        finish(html, false, WIDE).unwrap_or_else(|e| panic!("{e}"))
    }

    /// A page with `css` in its one author sheet.
    fn page(css: &str, body: &str) -> String {
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>t</title>\
             <style>{css}</style></head><body>{body}</body></html>"
        )
    }

    /// The sheet the pass added to `html`'s output, and the output without it,
    /// which must be the page as it was.
    fn added(html: &str) -> Option<String> {
        let (out, before) = (safe(html), plain(html));
        if out == before {
            return None;
        }
        let from = out.find("<style>@").unwrap_or_else(|| panic!("{out}"));
        let to = from
            + out[from..]
                .find("</style>")
                .unwrap_or_else(|| panic!("{out}"));
        let sheet = out[from + "<style>".len()..to].to_owned();
        let without = format!("{}{}", &out[..from], &out[to + "</style>".len()..]);
        assert_eq!(without, before, "only a sheet is added");
        Some(sheet)
    }

    /// The rules in the added sheet's layer.
    fn rules_of(html: &str) -> Vec<String> {
        let Some(sheet) = added(html) else {
            return Vec::new();
        };
        let sheet = sheet
            .strip_prefix("@namespace s url(http://www.w3.org/2000/svg);")
            .unwrap_or(&sheet);
        let inner = sheet
            .strip_prefix("@layer{")
            .and_then(|s| s.strip_suffix('}'))
            .unwrap_or_else(|| panic!("{sheet}"));
        inner.split_inclusive('}').map(str::to_owned).collect()
    }

    const SVG: &str = "<svg width=\"640\" height=\"208\" viewBox=\"0 0 640 208\"><rect/></svg>";

    fn size(name: &str, value: &str, css: &str) -> String {
        format!("s|svg[{name}=\"{value}\"]:not(s|svg *){{{name}:{css}}}")
    }

    #[test]
    fn an_svgs_size_is_written_below_every_author_rule() {
        let html = page("svg { display: block }", SVG);
        let out = safe(&html);
        // After what declares the encoding, before the page's first sheet,
        // and in a layer: the first layer is the lowest.
        assert!(
            out.contains(
                "<meta charset=\"utf-8\"><title>t</title><style>@namespace s \
                 url(http://www.w3.org/2000/svg);@layer{s|svg[width=\"640\"]:not(s|svg *)\
                 {width:640px}s|svg[height=\"208\"]:not(s|svg *){height:208px}}</style>\
                 <style>svg { display: block }</style>"
            ),
            "{out}"
        );
        // The element is as it was: an inline style would outrank author
        // rules that the attribute loses to.
        assert!(
            out.contains(&SVG.replace("<rect/>", "<rect></rect>")),
            "{out}"
        );
        assert!(!out.contains("style=\""), "{out}");
        assert_eq!(
            rules_of(&html),
            [
                size("width", "640", "640px"),
                size("height", "208", "208px")
            ]
        );
    }

    /// The reviewer's counter-example to an inline style: with the common reset
    /// `height: auto` has to win, as it does over the attribute.
    #[test]
    fn a_reset_still_wins_over_the_attributes() {
        let html = page(
            "svg{display:block;max-width:100%;height:auto} .box{width:200px}",
            &format!("<div class=\"box\">{SVG}</div><p>after</p>"),
        );
        let out = safe(&html);
        assert!(!out.contains("style=\""), "{out}");
        let sheet = added(&html).unwrap_or_else(|| panic!("{out}"));
        assert!(sheet.contains("@layer{"), "{sheet}");
        // The author's sheet follows the added one, untouched.
        assert!(out.contains("</style><style>svg{display:block;max-width:100%;height:auto}"));
    }

    #[test]
    fn lengths_are_svgs() {
        for (value, css) in [
            ("100", Some("100px")),
            ("0", Some("0px")),
            ("12.5", Some("12.5px")),
            ("007", Some("007px")),
            ("100px", Some("100px")),
            ("10mm", Some("10mm")),
            ("2.5pc", Some("2.5pc")),
            ("1in", Some("1in")),
            ("50%", None),
            ("2em", None),
            ("1e2", None),
            ("auto", None),
            ("-5", None),
            (" 100", None),
            ("100 ", None),
            ("100PX", None),
            ("1.", None),
            (".5", None),
            ("+5", None),
            ("12345678", None),
            ("1.23456", None),
            ("", None),
            ("１００", None),
        ] {
            assert_eq!(svg_length(value).as_deref(), css, "{value:?}");
            let html = page("", &format!("<svg width=\"{value}\"></svg>"));
            let expected: Vec<String> = css.iter().map(|c| size("width", value, c)).collect();
            assert_eq!(rules_of(&html), expected, "{value:?}");
        }
    }

    #[test]
    fn each_size_is_written_once_and_only_for_an_outermost_svg() {
        let html = page(
            "",
            "<svg width=\"300\" height=\"200\"><svg width=\"11\" height=\"12\"></svg>\
             <foreignObject><svg width=\"13\"></svg></foreignObject></svg>\
             <svg height=\"200\" width=\"300\"></svg><svg width=\"40\"></svg>\
             <math><svg width=\"77\" height=\"78\"></svg></math>\
             <div width=\"55\"></div><img width=\"66\" height=\"67\">",
        );
        assert_eq!(
            rules_of(&html),
            [
                size("width", "300", "300px"),
                size("height", "200", "200px"),
                size("width", "40", "40px"),
            ]
        );
    }

    #[test]
    fn an_animated_size_isnt_the_attributes() {
        let html = page(
            "",
            "<svg width=\"100\" height=\"50\"><set attributeName=\"width\" to=\"300\"/></svg>",
        );
        assert_eq!(added(&html), None);
    }

    /// Everything that lets a page's CSS see a new `<style>`, or hides CSS
    /// from this pass, leaves the page as it was.
    #[test]
    fn the_sheet_is_added_only_where_no_css_can_see_it() {
        // Nested deeper than this pass reads, in the text Chrome reads. To a
        // printer, which decodes references, it is one string.
        let deep = format!("a {{ b: &#39;{}x{}&#39; }}", "(".repeat(40), ")".repeat(40));
        let seen = [
            // CSS that isn't in the page.
            ("@import url(x.css);", ""),
            ("@media print { @import 'x.css'; }", ""),
            // The only way from `<head>` to what is rendered.
            ("html:has(style + style) p { color: red }", ""),
            ("p:not(:has(b)) { color: red }", ""),
            // What's in `<head>` shown.
            ("head, style { display: block }", ""),
            ("* { display: block }", ""),
            ("html > * { display: flex }", ""),
            (":not(body) { display: inline }", ""),
            ("head { all: unset }", ""),
            (".a { .b { display: block } }", ""),
            ("@keyframes k { from { display: block } }", ""),
            ("@scope (head) { display: block }", ""),
            ("x|head { display: block }", ""),
            (deep.as_str(), ""),
        ];
        for (css, extra) in seen {
            let html = page(css, &format!("{SVG}{extra}"));
            assert_eq!(safe(&html), plain(&html), "{css}");
        }
        let unseen = [
            "[hidden] { display: none !important }",
            "head { display: none }",
            ".grid, body > div { display: grid }",
            "html, body { display: block }",
            ":root { display: block }",
            "@media print { .no-print { display: none } p { display: block } }",
            "*, *::before, *::after { box-sizing: border-box; margin: 0 }",
            "@page { size: A4; margin: 15mm; @bottom-center { content: counter(page) } }",
            "@font-face { font-family: x; src: url(x.woff2); font-display: block }",
            "p:is(.a, .b) { color: red } /* :has( @import */ q { content: ':has(' }",
        ];
        for css in unseen {
            let html = page(css, SVG);
            assert_eq!(rules_of(&html).len(), 2, "{css}");
        }
    }

    #[test]
    fn a_linked_sheet_could_say_anything() {
        for link in [
            "<link rel=\"stylesheet\" href=\"a.css\">",
            "<link rel=\"alternate StyleSheet\" href=\"a.css\">",
            "<svg><link rel=\"stylesheet\" href=\"a.css\"></link></svg>",
        ] {
            let html = page("", &format!("{SVG}{link}<pre>a\n</pre>"));
            assert_eq!(safe(&html), plain(&html), "{link}");
        }
        let html = page("", &format!("{SVG}<link rel=\"icon\" href=\"a.png\">"));
        assert_eq!(rules_of(&html).len(), 2);
    }

    #[test]
    fn head_keeps_what_selectors_and_decoders_go_by() {
        // An empty `<head>` would stop matching `head:empty`.
        let empty = format!("<!doctype html><html><head></head><body>{SVG}</body></html>");
        assert_eq!(safe(&empty), plain(&empty));
        let text = format!("<!doctype html><html><head> </head><body>{SVG}</body></html>");
        assert_eq!(safe(&text), plain(&text));
        // Text that isn't ASCII, and nothing ahead of the sheet to say what it is.
        let undeclared = format!(
            "<!doctype html><html><head><title>売上</title></head><body>{SVG}</body></html>"
        );
        assert_eq!(safe(&undeclared), plain(&undeclared));
        let late = format!(
            "<!doctype html><html><head><style>p{{}}</style><meta charset=\"utf-8\"></head>\
             <body>売上{SVG}</body></html>"
        );
        assert_eq!(safe(&late), plain(&late));
        for meta in [
            "<meta charset=\"utf-8\">",
            "<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\">",
        ] {
            let declared = format!(
                "<!doctype html><html><head><title>売上</title>{meta}<base href=\"/\">\
                 <style>p{{}}</style></head><body>{SVG}</body></html>"
            );
            let out = safe(&declared);
            assert!(
                out.contains(&format!("{meta}<base href=\"/\"><style>@namespace")),
                "{out}"
            );
        }
        // ASCII is ASCII in any encoding.
        let ascii =
            format!("<!doctype html><html><head><title>t</title></head><body>{SVG}</body></html>");
        assert!(safe(&ascii).contains("<title>t</title><style>@namespace"));
    }

    #[test]
    fn a_page_that_keeps_its_scripts_is_left_alone() {
        let html = page(
            ".row { border-bottom: 1px solid #ccc }",
            &format!("{SVG}<pre>a\n</pre><script>document.head.firstChild</script>"),
        );
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::from_secs(600));
        let run = |keep_scripts, printer_safe| {
            let finish = Finish {
                keep_scripts,
                max_bytes: WIDE,
                printer_safe,
            };
            finish_page(&html, finish, None, &gate).unwrap_or_else(|e| panic!("{e}"))
        };
        assert_eq!(run(true, true), run(true, false));
        assert_ne!(run(false, true), run(false, false));
        // An island marker is data, and stays.
        let island = page(
            "",
            &format!("{SVG}<script type=\"application/vnd.surimono+json\">{{}}</script>"),
        );
        assert_eq!(rules_of(&island).len(), 2);
    }

    const ROW: &str = ".row { padding: 4px 0; border-bottom: 1px solid #d9e2ec; }";
    const ROWS: &str = "<div class=\"row\">a</div><div class=\"row\">b</div>";

    #[test]
    fn a_one_sided_border_gives_its_colour_to_the_top() {
        assert_eq!(
            rules_of(&page(ROW, ROWS)),
            [".row{border-top-color:#d9e2ec}"]
        );
        let css = "h1,\n  h2 > b { border-bottom: 2PX Solid RED }\n\
                   blockquote { border-left: 4px solid rgb(0, 0, 0 / 50%) }\n\
                   tr:not(:last-child) td { border-bottom: 1px dashed }\n\
                   a[href^=\"http\"] { border-bottom-color: hsl(10 20% 30%) ; border-bottom: thin dotted transparent }\n\
                   .x { border-right: solid #abc; border-left: #def double 3px }\n\
                   .last { border-bottom: none } .zero { border-bottom: 0 } .w { border-bottom: 1px }\n\
                   @media print { .row { border-bottom: 0 none } }";
        assert_eq!(
            rules_of(&page(css, ROWS)),
            [
                "h1, h2 > b{border-top-color:red}",
                "blockquote{border-top-color:rgb(0, 0, 0 / 50%)}",
                "tr:not(:last-child) td{border-top-color:currentcolor}",
                "a[href^=\"http\"]{border-top-color:hsl(10 20% 30%)}",
                "a[href^=\"http\"]{border-top-color:transparent}",
                ".x{border-top-color:#abc}",
                ".x{border-top-color:#def}",
            ]
        );
    }

    #[test]
    fn a_rule_that_sets_the_top_itself_is_not_copied() {
        let css = ".card { border: 1px solid #ccc; border-bottom: 3px solid red }\n\
                   .t { border-top: 0; border-bottom: 1px solid blue }\n\
                   .c { border-left: 1px solid green; border-color: black }\n\
                   .row { border-bottom: 1px solid #eee }";
        assert_eq!(rules_of(&page(css, ROWS)), [".row{border-top-color:#eee}"]);
    }

    /// Each of these could make the copy show in Chrome, or leave the colour
    /// that prints unknown: then no border is copied.
    #[test]
    fn borders_are_copied_only_where_chrome_cant_show_it() {
        let cases = [
            // A top border that paints with no colour of an author's.
            (".row { border-top-style: solid }", ROWS),
            (".box { border-style: solid; border-width: 1px }", ROWS),
            (
                "@keyframes k { from { border-block-start-style: solid } }",
                ROWS,
            ),
            // A colour read another way.
            (
                ".row span { border-top: 4px solid; border-top-color: inherit }",
                ROWS,
            ),
            ("td { border: inherit }", ROWS),
            ("div { border-top-color: revert-layer }", ROWS),
            ("p { color: revert-layer }", ROWS),
            // A border of the user agent's that the selector could reach.
            ("", "<fieldset class=\"row\">a</fieldset>"),
            ("", "<hr class=\"row\">"),
            ("", "<input class=\"row\">"),
            ("", "<div class=\"row\" popover>a</div>"),
            ("", "<svg class=\"row\"></svg>"),
            // A border of an attribute's.
            ("", "<table border=\"1\"><tr><td>a</td></tr></table>"),
            ("", "<table rules=\"rows\"><tr><td>a</td></tr></table>"),
            ("", "<img border=\"2\" src=\"a.png\">"),
            // A one-sided border whose colour isn't known here.
            (".x { border-bottom: 1px solid var(--line) }", ROWS),
            (".x { border-bottom: 1px solid red !important }", ROWS),
            (
                ".x { border-bottom: 1px solid color-mix(in srgb, red, blue) }",
                ROWS,
            ),
            (".x { border-bottom: 1px solid red blue }", ROWS),
            (".x { border-left-color: rgb(var(--r), 0, 0) }", ROWS),
            (
                "@media screen { .x { border-bottom: 1px solid red } }",
                ROWS,
            ),
            (".x { .y { border-bottom: 1px solid red } }", ROWS),
            (".x::after { border-bottom: 1px solid red }", ROWS),
            (".x:before { border-bottom: 1px solid red }", ROWS),
            ("x|a { border-bottom: 1px solid red }", ROWS),
            ("* { border-bottom: 1px solid red }", ROWS),
            (":not(p) { border-right: 1px solid red }", ROWS),
            (".x { border-inline-start: 1px solid red }", ROWS),
            (".x { border-block-end-color: red }", ROWS),
            (".x { all: unset }", ROWS),
            ("", "<p style=\"border-bottom: 1px solid red\">a</p>"),
            ("", "<p style=\"border-style: solid\">a</p>"),
            ("", "<p style=\"all: initial\">a</p>"),
        ];
        for (css, body) in cases {
            let html = page(&format!("{ROW}\n{css}"), body);
            assert_eq!(added(&html), None, "{css} {body}");
        }
        // The same sheet in a `<style>` that doesn't always apply.
        for attribute in ["media=\"screen\"", "title=\"alt\"", "type=\"text/x-other\""] {
            let html = page("", ROWS).replace("<style>", &format!("<style {attribute}>{ROW}"));
            assert_eq!(added(&html), None, "{attribute}");
        }
        // What can't paint a side, or sets all four, doesn't count.
        let fine = [
            ".row:last-child { border-bottom: none }",
            "@media print { .row { border-bottom: 0 } }",
            "hr { border: 0; height: 1px; background: #ccc }",
            "input { border: 1px solid #999; border-radius: 2px }",
            "table { border-collapse: collapse; border-spacing: 0 }",
            ".c { border-width: 2px; border-color: red green blue black }",
        ];
        for css in fine {
            let html = page(&format!("{ROW}\n{css}"), &format!("{ROWS}<hr><input>"));
            assert_eq!(rules_of(&html), [".row{border-top-color:#d9e2ec}"], "{css}");
        }
    }

    #[test]
    fn a_selector_is_copied_only_if_it_means_the_same_in_the_sheet() {
        assert!(copyable(".a > b:hover[x=\"y\"]"));
        for selector in ["", "a&b", "a<b", ".a\\:b", "s|a", "a{", "a;b", "é"] {
            assert!(!copyable(selector), "{selector}");
        }
        assert_eq!(on_one_line("a,\n\t b").as_deref(), Some("a, b"));
        assert_eq!(on_one_line("[x=\"a\nb\"]"), None);
        assert_eq!(on_one_line("[x=\"a b\"]").as_deref(), Some("[x=\"a b\"]"));
    }

    #[test]
    fn too_many_rules_are_none() {
        let mut css = String::new();
        for i in 0..=MAX_RULES {
            css.push_str(&format!(".r{i} {{ border-bottom: 1px solid #ccc }}\n"));
        }
        assert_eq!(added(&page(&css, ROWS)), None);
        let mut svgs = String::new();
        for i in 0..=MAX_RULES {
            svgs.push_str(&format!("<svg width=\"{i}\"></svg>"));
        }
        assert_eq!(added(&page("", &svgs)), None);
    }

    /// What `html` loses to the pass: the bytes of the plain output that aren't
    /// in the safe one, if the two differ by one cut.
    fn cut(html: &str) -> Option<String> {
        let (out, before) = (safe(html), plain(html));
        if out == before {
            return None;
        }
        let same = out
            .bytes()
            .zip(before.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let tail = out.len() - same;
        assert_eq!(out[same..], before[before.len() - tail..], "{out}");
        Some(before[same..before.len() - tail].to_owned())
    }

    #[test]
    fn a_newline_ending_a_pre_goes() {
        for html in [
            page("", "<pre>one\ntwo\n</pre><hr>"),
            page(
                "",
                "<pre><code class=\"language-js\">one\ntwo\n</code></pre>",
            ),
            page("", "<pre><code><span>one</span>\n<b>two\n</b></code></pre>"),
            page(
                "pre { white-space: pre-wrap; display: block }",
                "<pre>one\n</pre>",
            ),
            page("pre { white-space: pre-line }", "<pre>a\n</pre>"),
            page(
                "code { display: block; padding: 1em }",
                "<pre><code>a\n</code></pre>",
            ),
            page(
                ".clearfix::after { content: ''; display: table; clear: both }",
                "<pre>a\n</pre>",
            ),
            page(
                "*, *::before, *::after { box-sizing: border-box }",
                "<pre>a\n</pre>",
            ),
            page(
                "pre::before { content: '$ ' } div::after { content: 'x' }",
                "<pre>a\n</pre>",
            ),
            page(
                "p { display: inline } pre { display: flex !important }",
                "<pre>a\n</pre>",
            ),
            "<pre>a\n</pre>".to_owned(),
            page("", "<pre>売上\n</pre>"),
        ] {
            assert_eq!(cut(&html).as_deref(), Some("\n"), "{html}");
        }
        // The first newline of a `<pre>` is the parser's, and is still written.
        let html = page("", "<pre>\n\none\n</pre>");
        assert!(safe(&html).contains("<pre>\n\none</pre>"));
    }

    /// The reviewer's counter-examples, and the rest: a newline or a break that
    /// is a line of its own, or that something follows.
    #[test]
    fn a_break_that_chrome_renders_stays() {
        for body in [
            // The newline ends a line with nothing in it: Chrome has that line.
            "<pre>one\n\n</pre>",
            "<pre>\n\n</pre>",
            "<pre>one\n \n</pre>",
            "<pre>one\n\t\n</pre>",
            "<pre>one\u{200b}\n</pre>",
            "<pre><b>one</b>\n</pre>",
            // Something follows it.
            "<pre>one\n<span></span></pre>",
            "<pre>one\n<br></pre>",
            "<pre><code>one\n</code> </pre>",
            // Not plain inline elements.
            "<pre><q>one\n</q></pre>",
            "<pre><textarea>one\n</textarea></pre>",
            "<pre><div>one\n</div></pre>",
            "<pre><svg><text>one\n</text></svg></pre>",
            "<pre><my-code>one\n</my-code></pre>",
            // Not a `<pre>`, or not one Chrome renders.
            "<div style=\"white-space: pre\">one\n</div>",
            "<template><pre>one\n</pre></template>",
            // A break element is never touched: a selector can see it go.
            "<div>one<br></div>",
            "<div>one<br><br></div>",
            "<div><br></div>",
            "<p>one<br clear=\"all\"></p>",
        ] {
            let html = page("", body);
            assert_eq!(safe(&html), plain(&html), "{body}");
        }
        for css in [
            // The `<pre>` may not be a block of its own.
            "pre { display: inline }",
            "pre { display: contents }",
            "pre { display: var(--d) }",
            "pre { display: revert }",
            ".code { all: unset }",
            "* { display: inline }",
            "body > :not(p) { display: table-cell }",
            "div { pre { display: inline } }",
            "@keyframes k { to { display: inline } }",
            // Something may come after the newline.
            "pre::after { content: 'END' }",
            "pre:after { content: 'END' }",
            "code::after { content: '' }",
            ".code::after { content: attr(x) }",
            "::after { content: 'x' }",
            "*::after { all: unset }",
            ".x { &::after { content: 'x' } }",
            // The newline may become a space with a width.
            "pre { white-space-collapse: preserve-spaces }",
            "span { white-space: preserve-spaces }",
            // CSS that isn't here.
            "@import 'x.css';",
        ] {
            let html = page(css, "<pre class=\"code\"><code>one\n</code></pre>");
            assert_eq!(safe(&html), plain(&html), "{css}");
        }
        let styled = page("", "<pre style=\"display: inline\">one\n</pre>");
        assert_eq!(safe(&styled), plain(&styled));
        let fine = page("", "<pre style=\"display: block; color: red\">one\n</pre>");
        assert_eq!(cut(&fine).as_deref(), Some("\n"));
    }

    #[test]
    fn what_ends_a_line_with_content() {
        for c in ['a', '}', '。', '売', 'é', '😀', '!'] {
            assert!(visible(c), "{c:?}");
        }
        for c in [
            ' ', '\t', '\n', '\u{a0}', '\u{3000}', '\u{200b}', '\u{feff}', '\u{ad}', '\u{2028}',
            '\u{0}', '\u{7f}', '\u{202e}', '\u{fe0f}',
        ] {
            assert!(!visible(c), "{c:?}");
        }
    }

    /// The output is the same every time, is itself settled, and goes back to
    /// the page as it was when it would be over a limit.
    #[test]
    fn the_rewritten_page_is_settled_and_within_the_limits() {
        let html = page(
            &format!("svg {{ display: block }} {ROW}"),
            &format!("{SVG}{ROWS}<pre><code>a\n</code></pre>"),
        );
        let out = safe(&html);
        assert_eq!(out, safe(&html));
        assert_ne!(out, plain(&html));
        // Parsed and screened again, by the pass or without it, it stays.
        assert_eq!(plain(&out), out);
        assert_eq!(out.matches("<style>").count(), 2);
        assert!(
            out.contains(".row{border-top-color:#d9e2ec}}</style>"),
            "{out}"
        );
        assert!(out.contains("<code>a</code>"), "{out}");
        // No room for the sheet: the page is the plain one, not an error.
        let before = plain(&html);
        let tight = finish(&html, true, before.len()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(tight, before);
    }

    #[test]
    fn a_cancel_or_the_deadline_ends_the_pass() {
        let html = page("svg { display: block }", SVG);
        let finish = Finish {
            keep_scripts: false,
            max_bytes: WIDE,
            printer_safe: true,
        };
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::from_secs(600));
        let mut tree = Tree::parse(
            &plain(&html),
            super::super::TreeLimits {
                max_nodes: 1 << 20,
                max_depth: 512,
                max_attributes: 4_096,
            },
            &gate,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let stopped = AtomicBool::new(true);
        let cancelled = Gate::new(&stopped, Duration::from_secs(600));
        assert!(matches!(
            rewrite(&mut tree, true, &cancelled),
            Err(Error::Cancelled)
        ));
        let late = Gate::new(&stop, Duration::ZERO);
        assert!(matches!(
            rewrite(&mut tree, true, &late),
            Err(Error::PageTimeLimit)
        ));
        assert!(matches!(
            finish_page(&html, finish, None, &late),
            Err(Error::PageTimeLimit)
        ));
    }
}
