//! SVG output helpers: make chart-library SVG safe and deterministic to
//! inline into an HTML document.

mod css;
mod text;

pub use text::MAX_OUTLINE_CHARS;

use std::collections::HashSet;
use std::ops::Range;

use roxmltree::{Document, Node, ParsingOptions};

use crate::GfxError;
use crate::fonts::FontSet;
use crate::measure::TextMeasurer;

/// What [`sanitize_svg`] does with SVG `<text>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextMode {
    /// Replace every SVG `<text>` with `<path>` outlines of its glyphs in the
    /// registered fonts, shaped exactly as [`TextMeasurer::measure`] shapes
    /// them. Any PDF engine then prints what was measured, whatever fonts it
    /// has. The text stays as `aria-label` on the group that replaces it, but
    /// can no longer be selected or searched in the PDF.
    #[default]
    Outlines,
    /// Keep `<text>` as text (with its `font-family` rewritten to the
    /// registered families). Only right for a renderer that lays out inline
    /// SVG text with the registered fonts; fulgur 0.40 does not (it uses
    /// system fonts there), so with fulgur the printed labels can differ from
    /// the measured ones or go missing.
    Keep,
}

/// Options for [`sanitize_svg`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SvgOptions {
    /// What to do with `<text>`; outlines by default.
    pub text: TextMode,
}

impl SvgOptions {
    /// The defaults: text becomes outlines.
    pub fn new() -> Self {
        Self::default()
    }

    /// These options with `text` set.
    pub fn with_text(mut self, text: TextMode) -> Self {
        self.text = text;
        self
    }
}

/// Largest number of XML nodes [`sanitize_svg`] accepts.
pub const MAX_SVG_NODES: u32 = 1_000_000;

/// Deepest element nesting [`sanitize_svg`] accepts. The XML parser recurses
/// per level (about 2 KB of stack each in debug builds); chart SVG nests
/// about 15 deep, and 128 levels fit in a 256 KB thread stack.
pub const MAX_SVG_DEPTH: usize = 128;

/// Elements removed with everything inside them, wherever they appear.
const REMOVED_ELEMENTS: &[&str] = &[
    "script",
    "iframe",
    "frame",
    "frameset",
    "object",
    "embed",
    "applet",
    "link",
    "meta",
    "base",
    "portal",
    "noscript",
    "plaintext",
    "xmp",
    "noembed",
    "noframes",
    "handler",
    "listener",
];

/// HTML start tags that end SVG ("foreign") content when an HTML parser
/// meets them inside inline SVG (HTML spec, "parsing tokens in foreign
/// content"). Inside SVG they would make the HTML parser read the rest as
/// HTML, so they are removed outside `<foreignObject>`. `font` is on the
/// list (SVG fonts are obsolete) whatever its attributes.
const HTML_BREAKOUT: &[&str] = &[
    "b",
    "big",
    "blockquote",
    "body",
    "br",
    "center",
    "code",
    "dd",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "font",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "hr",
    "i",
    "img",
    "li",
    "listing",
    "menu",
    "meta",
    "nobr",
    "ol",
    "p",
    "pre",
    "ruby",
    "s",
    "small",
    "span",
    "strong",
    "strike",
    "sub",
    "sup",
    "table",
    "tt",
    "u",
    "ul",
    "var",
];

/// SMIL elements; removed when they animate a URL or an event handler.
const ANIMATIONS: &[&str] = &[
    "set",
    "animate",
    "animatemotion",
    "animatetransform",
    "animatecolor",
];

/// Attributes holding a URL that a browser would load or navigate to.
const URL_ATTRIBUTES: &[&str] = &[
    "href",
    "src",
    "srcset",
    "action",
    "formaction",
    "poster",
    "background",
    "codebase",
    "data",
    "ping",
    "lowsrc",
    "dynsrc",
    "longdesc",
    "cite",
    "manifest",
    "icon",
    "profile",
    "usemap",
    "archive",
    "classid",
    "base",
];

/// Presentation attributes parsed as CSS values that may hold `url()`.
const URL_PRESENTATION: &[&str] = &[
    "fill",
    "stroke",
    "clip-path",
    "mask",
    "filter",
    "marker",
    "marker-start",
    "marker-mid",
    "marker-end",
    "cursor",
];

/// Makes SVG from a chart library safe and deterministic to inline into HTML.
///
/// - Removes `<script>` and other active or external elements, event-handler
///   attributes (`on*`), `javascript:` URLs and every external reference
///   (`href`, `src`, `url()`, `@import`); `#fragment` and `data:image/` URLs
///   stay, and `<a>` keeps `http`, `https` and `mailto` links.
/// - Prefixes every `id` with `sm{island}-` and rewrites `url(#id)`,
///   `href="#id"`, `#id` selectors in `<style>`, id lists (`aria-labelledby`,
///   `aria-describedby`, `for`, ...) and SMIL `begin`/`end` references to
///   match, so several islands can share one page. ECharts' per-chart class
///   names (`zr0-cls-1`) get the same prefix in `class` and in `<style>`.
/// - Rewrites `font-family` (attribute, `style` and `<style>`, including the
///   `font` shorthand) to the registered families measurement used, so a
///   browser rendering the inlined SVG (the Chrome references) shapes text
///   with the same fonts. fulgur 0.40 does not: it prints inline SVG text
///   with system fonts, which is why island text is outlined (see `HANDOFF.md`).
/// - Drops `@font-face` rules (`src: local()` would reach system fonts), the
///   XML declaration, DOCTYPE, comments and processing instructions, turns
///   CDATA into escaped text, and removes elements whose names would end SVG
///   parsing in an HTML parser.
///
/// Rules in a kept `<style>` apply to the whole page once the SVG is inlined
/// (a `text {...}` rule styles every SVG text on the page). Ids and ECharts
/// classes are prefixed, but other selectors are left as written.
///
/// - With [`TextMode::Outlines`] (the default), replaces SVG `<text>` with
///   outlines of its glyphs in the measurer's fonts (see [`TextMode`]).
///
/// Everything else is copied byte for byte. Input that is not well-formed
/// XML with an `<svg>` root, uses a DTD or has more than [`MAX_SVG_NODES`]
/// nodes is rejected with [`GfxError::InvalidSvg`], as is SVG with more than
/// [`MAX_OUTLINE_CHARS`] characters of text to outline. Outlining text with
/// no registered fonts fails with [`GfxError::NoFonts`].
pub fn sanitize_svg(
    svg: &str,
    island: usize,
    measurer: &TextMeasurer,
    options: &SvgOptions,
) -> Result<String, GfxError> {
    let clean = sanitize(svg, island, measurer.fonts())?;
    match options.text {
        TextMode::Keep => Ok(clean),
        TextMode::Outlines => text::outline_text(&clean, measurer),
    }
}

/// The sanitising part of [`sanitize_svg`], text kept.
fn sanitize(svg: &str, island: usize, fonts: &FontSet) -> Result<String, GfxError> {
    check_depth(svg, MAX_SVG_DEPTH)?;
    let doc = Document::parse_with_options(
        svg,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_SVG_NODES,
            ..ParsingOptions::default()
        },
    )
    .map_err(|e| invalid(e.to_string()))?;
    let root = doc.root_element();
    if !root.tag_name().name().eq_ignore_ascii_case("svg") {
        return Err(invalid(format!(
            "the root element is <{}>, not <svg>",
            root.tag_name().name()
        )));
    }
    let prefix = format!("sm{island}-");

    // Pass 1: ids of the elements that stay.
    let mut ids = HashSet::new();
    walk(root, svg, |visit| {
        if let Visit::Element {
            node,
            removed: false,
            ..
        } = visit
        {
            if let Some(id) = node.attribute("id") {
                ids.insert(id.to_string());
            }
        }
        Ok(())
    })?;

    // Pass 2: edits.
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    walk(root, svg, |visit| {
        match visit {
            Visit::Element {
                node,
                removed: true,
                ..
            } => edits.push((node.range(), String::new())),
            Visit::Element { node, .. } => {
                attribute_edits(node, svg, &prefix, &ids, fonts, &mut edits)?;
                if node.tag_name().name().eq_ignore_ascii_case("style") {
                    style_edit(node, svg, &prefix, &ids, fonts, &mut edits)?;
                }
            }
            Visit::Other { node } => {
                if node.is_comment() || node.is_pi() {
                    edits.push((node.range(), String::new()));
                } else if node.is_text() {
                    text_edit(node, svg, &mut edits)?;
                }
            }
        }
        Ok(())
    })?;

    edits.sort_by_key(|(r, _)| (r.start, r.end));
    let whole = root.range();
    let mut out = String::with_capacity(whole.len() + 256);
    let mut pos = whole.start;
    for (r, new) in &edits {
        if r.start < pos || r.end > whole.end {
            return Err(invalid("overlapping rewrites (internal error)".into()));
        }
        out.push_str(&svg[pos..r.start]);
        out.push_str(new);
        pos = r.end;
    }
    out.push_str(&svg[pos..whole.end]);
    Ok(out)
}

/// Rejects nesting deeper than `limit` before the (recursive) parser sees
/// it. Counts tags the way an XML parser does: comments, CDATA and
/// processing instructions are skipped, quoted attribute values may hold `>`.
fn check_depth(s: &str, limit: usize) -> Result<(), GfxError> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    let skip_to = |from: usize, end: &str| s[from..].find(end).map(|k| from + k + end.len());
    while let Some(k) = s[i..].find('<') {
        i += k;
        let rest = &s[i..];
        let next = if rest.starts_with("<!--") {
            skip_to(i + 4, "-->")
        } else if rest.starts_with("<![CDATA[") {
            skip_to(i + 9, "]]>")
        } else if rest.starts_with("<?") {
            skip_to(i + 2, "?>")
        } else if rest.starts_with("<!") {
            return Err(invalid("DTDs are not supported".into()));
        } else if rest.starts_with("</") {
            if depth == 0 {
                return Err(invalid("unbalanced end tag".into()));
            }
            depth -= 1;
            skip_to(i + 2, ">")
        } else {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'>' {
                if b[j] == b'"' || b[j] == b'\'' {
                    let q = b[j];
                    j += 1;
                    while j < b.len() && b[j] != q {
                        j += 1;
                    }
                }
                j += 1;
            }
            if j < b.len() && b[j - 1] != b'/' {
                depth += 1;
                if depth > limit {
                    return Err(invalid(format!("elements nest deeper than {limit} levels")));
                }
            }
            Some(j + 1)
        };
        match next {
            Some(n) if n <= b.len() => i = n,
            // Unterminated markup: the parser reports it.
            _ => return Ok(()),
        }
    }
    Ok(())
}

fn invalid(reason: String) -> GfxError {
    GfxError::InvalidSvg { reason }
}

/// The namespace an HTML parser gives an element when the SVG is inlined into
/// an HTML body. It decides how the HTML parser reads the element's content,
/// which is what the sanitiser must agree with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ns {
    Html,
    Svg,
    MathMl,
}

enum Visit<'a, 'input> {
    Element {
        node: Node<'a, 'input>,
        /// The namespace an HTML parser puts this element in.
        ns: Ns,
        removed: bool,
    },
    Other {
        node: Node<'a, 'input>,
    },
}

/// Visits `root` and its descendants in document order without recursion.
/// Removed elements are visited, their contents are not; neither are the
/// contents of `<style>`, which [`style_edit`] handles as a whole.
fn walk<'a, 'input: 'a>(
    root: Node<'a, 'input>,
    input: &str,
    mut f: impl FnMut(Visit<'a, 'input>) -> Result<(), GfxError>,
) -> Result<(), GfxError> {
    // (node, the namespace an HTML parser inserts it in: what the HTML
    // spec's tree construction dispatcher decides from its parent)
    let mut stack = vec![(root, Ns::Html)];
    while let Some((node, context)) = stack.pop() {
        if !node.is_element() {
            f(Visit::Other { node })?;
            continue;
        }
        let local = node.tag_name().name().to_ascii_lowercase();
        let ns = match context {
            Ns::Html if local == "svg" => Ns::Svg,
            Ns::Html if local == "math" => Ns::MathMl,
            other => other,
        };
        let removed = is_removed(node, ns, input);
        f(Visit::Element { node, ns, removed })?;
        if removed || local == "style" {
            continue;
        }
        let child_context = children_context(node, ns, &local);
        let mut child = node.last_child();
        while let Some(c) = child {
            stack.push((c, child_context));
            child = c.prev_sibling();
        }
    }
    Ok(())
}

/// How an HTML parser reads an element's children: HTML integration points
/// (SVG `foreignObject`, `desc`, `title`; MathML `annotation-xml` with an
/// HTML encoding) and MathML text integration points (`mi`, `mo`, `mn`,
/// `ms`, `mtext`) switch back to HTML; other foreign elements keep their
/// namespace.
fn children_context(node: Node<'_, '_>, ns: Ns, local: &str) -> Ns {
    match ns {
        Ns::Html => Ns::Html,
        Ns::Svg if matches!(local, "foreignobject" | "desc" | "title") => Ns::Html,
        Ns::Svg => Ns::Svg,
        Ns::MathMl if matches!(local, "mi" | "mo" | "mn" | "ms" | "mtext") => Ns::Html,
        Ns::MathMl if local == "annotation-xml" => {
            let encoding = node
                .attributes()
                .find(|a| a.name().eq_ignore_ascii_case("encoding"))
                .map(|a| a.value().trim().to_ascii_lowercase());
            if matches!(
                encoding.as_deref(),
                Some("text/html" | "application/xhtml+xml")
            ) {
                Ns::Html
            } else {
                Ns::MathMl
            }
        }
        Ns::MathMl => Ns::MathMl,
    }
}

/// The element's qualified name as written, in lower case (as an HTML
/// parser would compare it).
fn qname_lower(node: Node<'_, '_>, input: &str) -> String {
    let start = node.range().start + 1;
    let rest = input.get(start..).unwrap_or("");
    let end = rest
        .find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
        .unwrap_or(rest.len());
    rest[..end].to_ascii_lowercase()
}

fn is_removed(node: Node<'_, '_>, ns: Ns, input: &str) -> bool {
    let local = node.tag_name().name().to_ascii_lowercase();
    if REMOVED_ELEMENTS.contains(&local.as_str()) {
        return true;
    }
    if ns != Ns::Html && HTML_BREAKOUT.contains(&qname_lower(node, input).as_str()) {
        return true;
    }
    if ANIMATIONS.contains(&local.as_str()) {
        let target = node
            .attributes()
            .find(|a| a.name().eq_ignore_ascii_case("attributeName"))
            .map(|a| a.value().trim().to_ascii_lowercase())
            .unwrap_or_default();
        let target = target.rsplit(':').next().unwrap_or("");
        if URL_ATTRIBUTES.contains(&target) || target.starts_with("on") || target == "style" {
            return true;
        }
    }
    false
}

/// True if the value, ignoring whitespace and controls, names a script URL.
fn has_script_scheme(value: &str) -> bool {
    let compact: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && !c.is_control())
        .flat_map(char::to_lowercase)
        .collect();
    ["javascript:", "vbscript:", "livescript:"]
        .iter()
        .any(|s| compact.contains(s))
}

enum AttrAction {
    Keep,
    Remove,
    Replace(String),
}

fn attribute_edits(
    node: Node<'_, '_>,
    input: &str,
    prefix: &str,
    ids: &HashSet<String>,
    fonts: &FontSet,
    edits: &mut Vec<(Range<usize>, String)>,
) -> Result<(), GfxError> {
    let element = node.tag_name().name().to_ascii_lowercase();
    for attr in node.attributes() {
        let local = attr.name().to_ascii_lowercase();
        let value = attr.value();
        let plain = attr.namespace().is_none();
        let (value_range, delim) = value_range(&attr, input)?;
        // Family names are quoted with the quote the attribute doesn't use.
        let quote = if delim == '"' { '\'' } else { '"' };
        let cx = css::CssContext {
            fonts,
            id_prefix: prefix,
            ids,
            quote,
        };
        let action = if local.starts_with("on") || has_script_scheme(value) {
            AttrAction::Remove
        } else if local == "id" && plain {
            AttrAction::Replace(format!("{prefix}{value}"))
        } else if ID_LIST_ATTRIBUTES.contains(&local.as_str()) && plain {
            match rewrite_id_list(value, prefix, ids) {
                Some(new) => AttrAction::Replace(new),
                None => AttrAction::Keep,
            }
        } else if (local == "begin" || local == "end") && plain {
            match rewrite_smil_times(value, prefix, ids) {
                Some(new) => AttrAction::Replace(new),
                None => AttrAction::Keep,
            }
        } else if local == "class" && plain {
            match rewrite_classes(value, prefix) {
                Some(new) => AttrAction::Replace(new),
                None => AttrAction::Keep,
            }
        } else if URL_ATTRIBUTES.contains(&local.as_str()) {
            let t = value.trim();
            if let Some(rest) = t.strip_prefix('#') {
                if local == "href" {
                    AttrAction::Replace(format!("#{prefix}{rest}"))
                } else {
                    AttrAction::Keep
                }
            } else if css::is_inline_image(t) || (element == "a" && local == "href" && is_link(t)) {
                AttrAction::Keep
            } else {
                AttrAction::Remove
            }
        } else if local == "font-family" && plain {
            match css::family_value(fonts, value, quote) {
                Some(new) => AttrAction::Replace(new),
                None => AttrAction::Keep,
            }
        } else if local == "style" && plain {
            match css::rewrite_declarations(value, &cx) {
                Ok(Some(new)) => AttrAction::Replace(new),
                Ok(None) => AttrAction::Keep,
                Err(_) => AttrAction::Remove,
            }
        } else if URL_PRESENTATION.contains(&local.as_str())
            || value.to_ascii_lowercase().contains("url(")
        {
            match css::rewrite_urls(value, &cx) {
                Ok(Some(new)) => AttrAction::Replace(new),
                Ok(None) => AttrAction::Keep,
                Err(_) => AttrAction::Remove,
            }
        } else {
            AttrAction::Keep
        };
        match action {
            AttrAction::Keep => {}
            AttrAction::Remove => {
                // Take the whitespace before the attribute with it.
                let r = attr.range();
                let bytes = input.as_bytes();
                let mut start = r.start;
                while start > 0 && matches!(bytes[start - 1], b' ' | b'\t' | b'\n' | b'\r') {
                    start -= 1;
                }
                edits.push((start..r.end, String::new()));
            }
            AttrAction::Replace(new) => {
                edits.push((value_range, escape_attribute(&new, delim)));
            }
        }
    }
    Ok(())
}

/// Attributes holding a space-separated list of ids (ARIA relations, HTML
/// `for`, `headers`, `list`, `form` inside `<foreignObject>`).
const ID_LIST_ATTRIBUTES: &[&str] = &[
    "aria-activedescendant",
    "aria-controls",
    "aria-describedby",
    "aria-details",
    "aria-errormessage",
    "aria-flowto",
    "aria-labelledby",
    "aria-owns",
    "for",
    "form",
    "headers",
    "list",
];

/// Prefixes the tokens of a space-separated id list that name ids defined in
/// this SVG. `None`: nothing to change.
fn rewrite_id_list(value: &str, prefix: &str, ids: &HashSet<String>) -> Option<String> {
    let mut out = String::with_capacity(value.len() + 16);
    let mut changed = false;
    let mut rest = value;
    while !rest.is_empty() {
        let token_len = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let (token, after) = rest.split_at(token_len);
        if !token.is_empty() && ids.contains(token) {
            out.push_str(prefix);
            changed = true;
        }
        out.push_str(token);
        let ws_len = after
            .find(|c: char| !c.is_ascii_whitespace())
            .unwrap_or(after.len());
        out.push_str(&after[..ws_len]);
        rest = &after[ws_len..];
    }
    changed.then_some(out)
}

/// Prefixes the element ids in a SMIL `begin`/`end` list (`a.click+1s;
/// b.end`) that name ids defined in this SVG. `None`: nothing to change.
fn rewrite_smil_times(value: &str, prefix: &str, ids: &HashSet<String>) -> Option<String> {
    let mut out = String::with_capacity(value.len() + 16);
    let mut changed = false;
    for (n, part) in value.split(';').enumerate() {
        if n > 0 {
            out.push(';');
        }
        let trimmed = part.trim_start();
        let lead = part.len() - trimmed.len();
        let id_len = trimmed.find('.').unwrap_or(0);
        if id_len > 0 && ids.contains(&trimmed[..id_len]) {
            out.push_str(&part[..lead]);
            out.push_str(prefix);
            out.push_str(trimmed);
            changed = true;
        } else {
            out.push_str(part);
        }
    }
    changed.then_some(out)
}

/// True for ECharts' per-chart class names (`zr0-cls-1`): `zr`, digits, `-`.
/// Every ECharts island names its classes from `zr0`, so without a prefix
/// one island's `<style>` would style another's elements.
pub(crate) fn is_echarts_class(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("zr") else {
        return false;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && rest.as_bytes().get(digits) == Some(&b'-')
}

/// Prefixes ECharts' class names in a `class` value. `None`: nothing to change.
fn rewrite_classes(value: &str, prefix: &str) -> Option<String> {
    let mut out = String::with_capacity(value.len() + 16);
    let mut changed = false;
    let mut rest = value;
    while !rest.is_empty() {
        let token_len = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let (token, after) = rest.split_at(token_len);
        if is_echarts_class(token) {
            out.push_str(prefix);
            changed = true;
        }
        out.push_str(token);
        let ws_len = after
            .find(|c: char| !c.is_ascii_whitespace())
            .unwrap_or(after.len());
        out.push_str(&after[..ws_len]);
        rest = &after[ws_len..];
    }
    changed.then_some(out)
}

/// Navigation links that load nothing by themselves.
fn is_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|s| lower.starts_with(s))
}

/// The attribute value's byte range without quotes, and its quote, checked
/// against the source (roxmltree's ranges assume short names and spacing).
fn value_range(
    attr: &roxmltree::Attribute<'_, '_>,
    input: &str,
) -> Result<(Range<usize>, char), GfxError> {
    let full = attr.range();
    let r = attr.range_value();
    let b = input.as_bytes();
    let ok = r.start > full.start
        && r.start <= r.end
        && r.end < b.len()
        && full.end == r.end + 1
        && matches!(b[r.start - 1], b'"' | b'\'')
        && b[r.end] == b[r.start - 1];
    if !ok {
        return Err(invalid(format!(
            "attribute {:?} is laid out in a way that can't be rewritten safely",
            attr.name()
        )));
    }
    Ok((r.clone(), b[r.start - 1] as char))
}

/// Rewrites a `<style>` element as one unit.
///
/// The style sheet is the concatenation of the element's text children (a
/// browser joins them; scanning them one by one would let a string opened in
/// one hide a `url()` in the next). Comments go. An element child makes the
/// whole `<style>` go: in HTML (inside `<foreignObject>`, `<desc>`, ...) a
/// style's content is raw text, so a child element's markup would be CSS.
///
/// The rewritten CSS is written as plain text, never with entities: inside an
/// HTML integration point an HTML parser does not decode entities in
/// `<style>`, so `&#34;` would hide a quote from this scanner while the browser
/// sees none, and an added `&amp;` would add a `;` the scanner never saw.
/// CSS that would need escaping in XML (`&`, `<` or `]]>`), or that ends
/// inside a string or comment, is dropped.
fn style_edit(
    node: Node<'_, '_>,
    input: &str,
    prefix: &str,
    ids: &HashSet<String>,
    fonts: &FontSet,
    edits: &mut Vec<(Range<usize>, String)>,
) -> Result<(), GfxError> {
    let Some(first) = node.first_child() else {
        return Ok(());
    };
    if node.children().any(|c| c.is_element()) {
        edits.push((node.range(), String::new()));
        return Ok(());
    }
    let css: String = node
        .children()
        .filter(|c| c.is_text())
        .filter_map(|c| c.text())
        .collect();
    let start = first.range().start;
    let end = input[..node.range().end]
        .rfind("</")
        .filter(|&e| e >= start)
        .ok_or_else(|| invalid("style element range (internal error)".into()))?;
    let raw = &input[start..end];
    let cx = css::CssContext {
        fonts,
        id_prefix: prefix,
        ids,
        quote: '"',
    };
    let rewritten = css::rewrite_stylesheet(&css, &cx).map_err(invalid)?;
    let out = rewritten.as_deref().unwrap_or(&css);
    if out.contains(['&', '<']) || out.contains("]]>") || !css::strings_and_comments_closed(out) {
        edits.push((start..end, String::new()));
    } else if out != raw {
        edits.push((start..end, out.to_string()));
    }
    Ok(())
}

/// Turns a text node holding CDATA into escaped text. (Text in `<style>` is
/// handled by [`style_edit`] and never reaches here.)
fn text_edit(
    node: Node<'_, '_>,
    input: &str,
    edits: &mut Vec<(Range<usize>, String)>,
) -> Result<(), GfxError> {
    // roxmltree merges text and CDATA into one node but reports only the
    // first chunk's range; the node really runs to the next sibling or to
    // the parent's end tag.
    let start = node.range().start;
    let end = match node.next_sibling() {
        Some(next) => next.range().start,
        None => {
            let parent_end = node.parent().map(|p| p.range().end).unwrap_or(input.len());
            input[..parent_end].rfind('<').unwrap_or(parent_end)
        }
    };
    if end < start {
        return Err(invalid("text node range (internal error)".into()));
    }
    let raw = &input[start..end];
    if raw.contains("<![CDATA[") {
        edits.push((start..end, escape_text(node.text().unwrap_or(""))));
    }
    Ok(())
}

fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

fn escape_attribute(s: &str, delim: char) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' if delim == '"' => out.push_str("&quot;"),
            '\'' if delim == '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(svg: &str) -> String {
        sanitize(svg, 0, &FontSet::new()).unwrap_or_else(|e| panic!("{svg}: {e}"))
    }

    const NS: &str =
        r#"xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink""#;

    fn svg(body: &str) -> String {
        format!("<svg {NS}>{body}</svg>")
    }

    #[test]
    fn untouched_svg_is_byte_identical() {
        let s = svg(
            "\n<g transform=\"translate(1 2)\"  class='a'>\n  <path d=\"M0 0L1 1\" fill=\"#fff\"/>\
             <text x='1' xml:space=\"preserve\">a &amp; b &lt; c</text></g>\n",
        );
        assert_eq!(clean(&s), s);
    }

    #[test]
    fn strips_scripts_handlers_and_script_urls() {
        let s = svg(
            r#"<script>alert(1)</script><svg:script xmlns:svg="http://www.w3.org/2000/svg">x</svg:script><rect onload="alert(1)" ONCLICK='x' width="1"/><a href="javascript:alert(1)"><text>t</text></a><a xlink:href=" java&#9;script:alert(1)">u</a>"#,
        );
        assert_eq!(
            clean(&s),
            svg(r#"<rect width="1"/><a><text>t</text></a><a>u</a>"#)
        );
    }

    #[test]
    fn strips_external_references() {
        let s = svg(
            r##"<image href="https://evil.example/x.png" width="1"/><use xlink:href="other.svg#a"/><image href="data:image/png;base64,AA"/><rect fill="url(http://x/#g)" stroke="url(#g)"/><a href="https://example.com">k</a><foreignObject><iframe src="https://x"></iframe><img xmlns="http://www.w3.org/1999/xhtml" src="https://x/y.png" alt="a"/></foreignObject>"##,
        );
        assert_eq!(
            clean(&s),
            svg(
                r##"<image width="1"/><use/><image href="data:image/png;base64,AA"/><rect fill="none" stroke="url(#sm0-g)"/><a href="https://example.com">k</a><foreignObject><img xmlns="http://www.w3.org/1999/xhtml" alt="a"/></foreignObject>"##
            )
        );
    }

    #[test]
    fn prefixes_ids_and_references() {
        let s = svg(
            r##"<defs><linearGradient id="g0"/><clipPath id='c0'/></defs><path fill="url(#g0)" clip-path="url('#c0')"/><use href="#g0"/><use xlink:href="#c0"/><style>#g0{fill:url(#c0)} #other{}</style>"##,
        );
        let out = sanitize(&s, 3, &FontSet::new()).unwrap();
        assert_eq!(
            out,
            svg(
                r##"<defs><linearGradient id="sm3-g0"/><clipPath id='sm3-c0'/></defs><path fill="url(#sm3-g0)" clip-path="url(#sm3-c0)"/><use href="#sm3-g0"/><use xlink:href="#sm3-c0"/><style>#sm3-g0{fill:url(#sm3-c0)} #other{}</style>"##
            )
        );
    }

    #[test]
    fn drops_prolog_comments_pis_and_cdata() {
        let s = format!(
            "<?xml version=\"1.0\"?>\n<!-- c --><svg {NS}><!-- c --><?pi x?><text><![CDATA[a<b&c]]>d</text><style><![CDATA[.a>.b{{}}]]>\n</style></svg>\n"
        );
        assert_eq!(
            clean(&s),
            svg("<text>a&lt;b&amp;cd</text><style>.a>.b{}\n</style>")
        );
    }

    #[test]
    fn removes_html_breakout_elements_outside_foreign_object() {
        let s = svg(
            r#"<p><style>x</style></p><font face="x">f</font><g><SPAN>s</SPAN></g><foreignObject><div xmlns="http://www.w3.org/1999/xhtml"><p>ok</p></div></foreignObject>"#,
        );
        assert_eq!(
            clean(&s),
            svg(
                r#"<g></g><foreignObject><div xmlns="http://www.w3.org/1999/xhtml"><p>ok</p></div></foreignObject>"#
            )
        );
    }

    #[test]
    fn removes_animations_of_urls_and_handlers() {
        let s = svg(
            r#"<a><animate attributeName="href" to="https://x"/><set attributeName="xlink:href" to="x"/><set attributeName="onclick" to="x"/><animate attributeName="opacity" to="1"/></a>"#,
        );
        assert_eq!(
            clean(&s),
            svg(r#"<a><animate attributeName="opacity" to="1"/></a>"#)
        );
    }

    #[test]
    fn rejects_what_it_cannot_parse() {
        for bad in [
            "",
            "not xml",
            "<html/>",
            "<svg>",
            "<!DOCTYPE svg [<!ENTITY a \"b\">]><svg>&a;</svg>",
            "<svg>&nbsp;</svg>",
            "<svg><a xlink:href='#x'/></svg>",
            "<svg><!-- --!><img src=x onerror=alert(1)> --></svg>",
            "<p><svg/></p>",
        ] {
            assert!(
                matches!(
                    sanitize(bad, 0, &FontSet::new()),
                    Err(GfxError::InvalidSvg { .. })
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn deep_nesting_is_rejected_before_parsing() {
        let nest = |depth: usize, open: &str| {
            format!("<svg>{}{}</svg>", open.repeat(depth), "</g>".repeat(depth))
        };
        let ok = nest(MAX_SVG_DEPTH - 1, "<g a='>'>");
        assert_eq!(clean(&ok), ok);
        for s in [
            nest(MAX_SVG_DEPTH, "<g>"),
            nest(200_000, "<g>"),
            nest(200_000, "<g a=\"/>\">"),
            nest(200_000, "<!--x--><g><![CDATA[<g/>]]>"),
        ] {
            assert!(matches!(
                sanitize(&s, 0, &FontSet::new()),
                Err(GfxError::InvalidSvg { .. })
            ));
        }
        // Self-closing tags don't nest.
        let flat = format!("<svg>{}</svg>", "<g/>".repeat(10_000));
        assert_eq!(clean(&flat), flat);
    }

    /// In an HTML integration point a `<style>` is raw text: entities are not
    /// decoded there, so the sanitiser never writes them (review of #12).
    #[test]
    fn styles_never_hide_css_behind_entities() {
        let evil = "*{background:url(https://evil.example/x.png)}";
        for wrap in [
            "<desc>{}</desc>",
            "<title>{}</title>",
            "<foreignObject>{}</foreignObject>",
            "<foreignObject><math xmlns=\"http://www.w3.org/1998/Math/MathML\"><mi>{}</mi></math></foreignObject>",
            "<g>{}</g>",
        ] {
            for quote in ["&#34;", "&#39;", "&#47;&#42;", "&quot;"] {
                let style = format!("<style>.x{{a:{quote}}}{evil}</style>");
                let s = svg(&wrap.replace("{}", &style));
                let out = clean(&s);
                // The browser must see exactly the text the scanner saw: no
                // entity is written into a <style>, and a sheet left inside
                // an open string or comment is dropped.
                assert!(!out.contains('&'), "{s}\n -> {out}");
                assert!(!out.contains("evil.example"), "{s}\n -> {out}");
            }
        }
    }

    #[test]
    fn style_text_is_scanned_as_one_sheet() {
        // A string opened in one text node and closed in the next, around a
        // comment or a child element, must not hide a url().
        let s = svg(
            "<style>.x{a:\"<!-- -->\"}*{background:url(https://evil.example/1)}</style>\
             <foreignObject><style>.y{a:'<i b=\"'background:url(https://evil.example/2);'\"/>}</style></foreignObject>",
        );
        let out = clean(&s);
        assert_eq!(
            out,
            svg("<style>.x{a:\"\"}*{background:none}</style><foreignObject></foreignObject>")
        );
    }

    #[test]
    fn style_text_needing_escapes_is_dropped() {
        let s = svg(
            "<style>a&lt;b{fill:red} .c{fill:blue}</style><style>.d{}</style><style>x&amp;y{}</style>",
        );
        assert_eq!(
            clean(&s),
            svg("<style></style><style>.d{}</style><style></style>")
        );
        let s = svg("<style>a > b {fill:red}</style><style>a&gt;b{}</style>");
        assert_eq!(
            clean(&s),
            svg("<style>a > b {fill:red}</style><style>a>b{}</style>"),
            "a literal > is fine in XML and HTML"
        );
    }

    #[test]
    fn integration_points_follow_the_html_parser() {
        // desc and title hold HTML: a <p> there doesn't leave SVG, so it stays.
        let s = svg("<desc><p>x</p></desc><title><b>t</b></title>");
        assert_eq!(clean(&s), s);
        // MathML text integration points only count inside MathML: in SVG,
        // <math><mi> is an SVG element and a <p> under it breaks out.
        let s = svg("<math><mi><p>x</p></mi></math>");
        assert_eq!(clean(&s), svg("<math><mi></mi></math>"));
        let s = svg(
            "<foreignObject><math xmlns=\"http://www.w3.org/1998/Math/MathML\"><mi><p>x</p></mi><mrow><p>y</p></mrow></math></foreignObject>",
        );
        assert_eq!(
            clean(&s),
            svg(
                "<foreignObject><math xmlns=\"http://www.w3.org/1998/Math/MathML\"><mi><p>x</p></mi><mrow></mrow></math></foreignObject>"
            )
        );
    }

    #[test]
    fn id_lists_smil_times_and_echarts_classes_are_prefixed() {
        let s = svg(
            "<text id=\"t\" aria-labelledby=\"t  other\" aria-describedby=\"d\">x</text><desc id=\"d\"/>\
             <rect id=\"r\"><animate attributeName=\"opacity\" begin=\"r.click+1s; 2s;r.end\" end=\"other.end\"/></rect>\
             <foreignObject><label xmlns=\"http://www.w3.org/1999/xhtml\" for=\"t\">l</label></foreignObject>\
             <path class=\"zr0-cls-1 keep zr1-x\"/>",
        );
        assert_eq!(
            sanitize(&s, 4, &FontSet::new()).unwrap(),
            svg(
                "<text id=\"sm4-t\" aria-labelledby=\"sm4-t  other\" aria-describedby=\"sm4-d\">x</text><desc id=\"sm4-d\"/>\
                 <rect id=\"sm4-r\"><animate attributeName=\"opacity\" begin=\"sm4-r.click+1s; 2s;sm4-r.end\" end=\"other.end\"/></rect>\
                 <foreignObject><label xmlns=\"http://www.w3.org/1999/xhtml\" for=\"sm4-t\">l</label></foreignObject>\
                 <path class=\"sm4-zr0-cls-1 keep sm4-zr1-x\"/>"
            )
        );
        assert!(is_echarts_class("zr0-cls-1"));
        assert!(
            !is_echarts_class("zr-1") && !is_echarts_class("zrx-1") && !is_echarts_class("zr0")
        );
    }

    #[test]
    fn escaping() {
        assert_eq!(escape_text("a<b>&"), "a&lt;b&gt;&amp;");
        assert_eq!(escape_attribute("a\"b'c", '"'), "a&quot;b'c");
        assert_eq!(escape_attribute("a\"b'c", '\''), "a\"b&apos;c");
        assert!(has_script_scheme(" JaVa\tScRiPt:x"));
        assert!(!has_script_scheme("javascript"));
    }
}
