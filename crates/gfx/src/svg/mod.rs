//! SVG output helpers: make chart-library SVG safe and deterministic to
//! inline into an HTML document.

mod css;

use std::collections::HashSet;
use std::ops::Range;

use roxmltree::{Document, Node, ParsingOptions};

use crate::GfxError;
use crate::fonts::FontSet;

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
    // An island can't affect anything outside its own box: a <style> applies
    // to the whole page once inlined (ECharts' holds only hover rules), and
    // HTML content can restyle, re-parent or swallow the page around it.
    "style",
    "foreignobject",
    // SMIL animation: fulgur and usvg ignore it (the PDF shows the first
    // frame), ECharts and Vega emit none, and it can set any attribute or
    // property, URLs included, after sanitising.
    "set",
    "animate",
    "animatemotion",
    "animatetransform",
    "animatecolor",
    "discard",
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

/// Attributes the root `<svg>` keeps (lower case, no namespace), besides
/// [`ROOT_ROLES`], [`ROOT_ARIA`], `xml:lang`, `xml:space` and
/// [`ROOT_PROPERTIES`]. The root is
/// laid out by the page as a box of its own: nothing may move it out of
/// that box, put it over other content or let it paint outside (`position`,
/// `transform`, `overflow`, `filter`, `display`, ... all go).
const ROOT_ATTRIBUTES: &[&str] = &[
    "width",
    "height",
    "viewbox",
    "preserveaspectratio",
    "version",
    "baseprofile",
    "id",
    "lang",
    "style",
    "focusable",
];

/// `role` values the root keeps, and its ARIA attributes. The page's own CSS
/// can match the root's attributes (utility classes such as Tailwind's
/// `fixed inset-0`, or `[role=dialog]` rules), so the root keeps no `class`
/// and only roles and ARIA a chart needs.
const ROOT_ROLES: &[&str] = &[
    "img",
    "graphics-document",
    "graphics-object",
    "document",
    "figure",
    "group",
    "presentation",
    "none",
];
const ROOT_ARIA: &[&str] = &[
    "aria-label",
    "aria-labelledby",
    "aria-describedby",
    "aria-roledescription",
    "aria-hidden",
];

/// CSS properties the root `<svg>` keeps, in `style` or as presentation
/// attributes: paint and text settings its content inherits.
const ROOT_PROPERTIES: &[&str] = &[
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "color",
    "background-color",
    "opacity",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "font-variant",
    "font-stretch",
    "letter-spacing",
    "word-spacing",
    "text-anchor",
    "dominant-baseline",
    "text-rendering",
    "shape-rendering",
    "color-interpolation",
    "color-interpolation-filters",
    "paint-order",
    "user-select",
    "-webkit-user-select",
    "pointer-events",
];

/// Presentation attributes parsed as CSS values that may load an image
/// (`url()`, `image-set()`, ...). Their values get the same checks as a
/// declaration in a style sheet.
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
/// An island can't run or load anything, and can't affect anything outside
/// its own box:
///
/// - Removes `<script>` and other active or external elements, every
///   `<style>` (its rules would apply to the whole page), `<foreignObject>`
///   (HTML inside it can restyle, re-parent or swallow the page), all SMIL
///   animation (`<set>`, `<animate>`, `<animateMotion>`,
///   `<animateTransform>`, `<animateColor>`, `<discard>`), event-handler
///   attributes (`on*`), `javascript:` URLs and every external reference
///   (`href`, `src`, `url()`, `image-set()`); `#fragment` and `data:image/`
///   URLs stay, and `<a>` keeps `http`, `https` and `mailto` links.
/// - Keeps only text in `<desc>` and `<title>`, which an HTML parser reads
///   as HTML, and removes elements whose names would end SVG parsing in an
///   HTML parser (`<p>`, `<div>`, `<img>`, ...).
/// - Keeps only a short list of attributes on the root `<svg>` (size,
///   `viewBox`, `id`, a chart `role`, ARIA labels, inherited paint and font
///   settings; no `class`, which the page's own CSS could match), and only
///   those properties in its `style`: nothing that positions, transforms,
///   hides, filters or un-clips the island's box.
/// - Decides every URL as the URL Standard's parser reads it (only leading
///   and trailing C0 controls and spaces stripped, tab, LF and CR removed;
///   C1 controls refused), and writes kept URLs as the XML parser read them.
/// - Removes a `<use>` whose target is or holds a `<use>` (such chains fan
///   out exponentially in the printer), id-list tokens (`aria-labelledby`,
///   ...) that name no id of the SVG, and any `data-surimono-*` attribute.
/// - Reads all remaining CSS (`style` attributes and presentation attributes
///   such as `fill` or `mask`) with a CSS Syntax Level 3 tokenizer, as a
///   browser does. A declaration or attribute with a function that loads
///   without `url(` (`image-set()`, `image()`, `src()`, ...), an at-rule, an
///   escape in a name, or a string left open is dropped.
/// - Prefixes every `id` with `sm{island}-` and rewrites `url(#id)`,
///   `href="#id"` and id lists (`aria-labelledby`, `aria-describedby`, ...)
///   to match, so several islands can share one page. ECharts' per-chart
///   class names (`zr0-cls-1`) get the same prefix.
/// - Rewrites `font-family` (attribute and `style`, including the `font`
///   shorthand) to the registered families measurement used, so a browser
///   rendering the inlined SVG (the Chrome references) shapes text with the
///   same fonts. fulgur 0.40 does not: it prints inline SVG text with system
///   fonts, which is why island text is outlined (see `HANDOFF.md`).
/// - Drops the XML declaration, DOCTYPE, comments and processing
///   instructions, and turns CDATA into escaped text.
///
/// Everything else is copied byte for byte, except that a CSS-bearing
/// attribute whose raw bytes an HTML parser would read differently from the
/// XML parser (a literal tab or newline, an entity) is written again from
/// what was checked. Input that is not well-formed XML with an unprefixed
/// `<svg>` root, uses a DTD or has more than [`MAX_SVG_NODES`] nodes is
/// rejected with [`GfxError::InvalidSvg`].
pub fn sanitize_svg(svg: &str, island: usize, fonts: &FontSet) -> Result<String, GfxError> {
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
    // As written: an HTML parser reads `<svg:svg>` as an unknown HTML
    // element and everything inside it as HTML.
    let root_name = qname_lower(root, svg);
    if root_name != "svg" {
        return Err(invalid(format!(
            "the root element is <{root_name}>, not <svg>"
        )));
    }
    let prefix = format!("sm{island}-");

    // Pass 1: ids of the elements that stay, and those that are or hold a
    // <use>.
    let mut ids = HashSet::new();
    let mut ids_with_use = HashSet::new();
    walk(root, svg, |visit| {
        if let Visit::Element {
            node,
            removed: false,
            ..
        } = visit
        {
            if let Some(id) = node.attribute("id") {
                ids.insert(id.to_string());
                if node.descendants().any(is_use) {
                    ids_with_use.insert(id.to_string());
                }
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
            // A <use> of something that holds a <use> goes: chains of them
            // fan out exponentially in the printer (ten levels of ten uses
            // are 10^10 copies), after every script limit has ended.
            Visit::Element { node, .. }
                if is_use(node) && use_target(node).is_some_and(|t| ids_with_use.contains(t)) =>
            {
                edits.push((node.range(), String::new()))
            }
            Visit::Element { node, .. } => {
                let is_root = node == root;
                attribute_edits(node, is_root, svg, &prefix, &ids, fonts, &mut edits)?;
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
    /// Inside an HTML integration point (`desc`, `title`): removed.
    Html,
    Svg,
}

enum Visit<'a, 'input> {
    Element {
        node: Node<'a, 'input>,
        removed: bool,
    },
    Other {
        node: Node<'a, 'input>,
    },
}

/// Visits `root` (an `<svg>`) and its descendants in document order without
/// recursion. Removed elements are visited, their contents are not.
///
/// An HTML parser reads the children of SVG `desc` and `title` (HTML
/// integration points) as HTML, where a `<textarea>`, `<select>`, `<body>`
/// or a self-closing `<div/>` would swallow, re-parent or restyle the page
/// around the island. So they hold text only: every element in them is
/// removed. (`foreignObject`, the third integration point, is removed whole,
/// and MathML can't be reached without one.)
fn walk<'a, 'input: 'a>(
    root: Node<'a, 'input>,
    input: &str,
    mut f: impl FnMut(Visit<'a, 'input>) -> Result<(), GfxError>,
) -> Result<(), GfxError> {
    let mut stack = vec![(root, Ns::Svg)];
    while let Some((node, ns)) = stack.pop() {
        if !node.is_element() {
            f(Visit::Other { node })?;
            continue;
        }
        let removed = ns == Ns::Html || is_removed(node, input);
        f(Visit::Element { node, removed })?;
        if removed {
            continue;
        }
        let local = node.tag_name().name().to_ascii_lowercase();
        let child_ns = if matches!(local.as_str(), "desc" | "title") {
            Ns::Html
        } else {
            Ns::Svg
        };
        let mut child = node.last_child();
        while let Some(c) = child {
            stack.push((c, child_ns));
            child = c.prev_sibling();
        }
    }
    Ok(())
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

fn is_use(node: Node<'_, '_>) -> bool {
    node.is_element() && node.tag_name().name().eq_ignore_ascii_case("use")
}

/// The id a `<use>` points at (`href` or `xlink:href`, `#id`).
fn use_target<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    node.attributes()
        .filter(|a| a.name().eq_ignore_ascii_case("href"))
        .find_map(|a| {
            let v = a.value();
            let start = v.len() - v.trim_start_matches(|c: char| c <= ' ').len();
            v[start..]
                .strip_prefix('#')
                .map(|id| id.trim_end_matches(|c: char| c <= ' '))
        })
}

fn is_removed(node: Node<'_, '_>, input: &str) -> bool {
    let local = node.tag_name().name().to_ascii_lowercase();
    REMOVED_ELEMENTS.contains(&local.as_str())
        || HTML_BREAKOUT.contains(&qname_lower(node, input).as_str())
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
    is_root: bool,
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
            quote,
        };
        // Raw bytes of a value the browser parses as CSS are only copied when
        // they say exactly what the XML parser read: XML turns a literal tab
        // or newline in an attribute into a space, HTML keeps it, and a
        // newline ends a CSS string. Otherwise the value is written again.
        let raw = &input[value_range.clone()];
        let keep_as_read = |v: &str| {
            if escape_attribute(v, delim) == raw {
                AttrAction::Keep
            } else {
                AttrAction::Replace(v.to_string())
            }
        };
        let action = if local.starts_with("on")
            || local.starts_with("data-surimono-")
            || has_script_scheme(value)
            || (is_root && !root_keeps(&attr))
        {
            // `data-surimono-*` is what outlining writes for the checks: an
            // island can't forge it.
            AttrAction::Remove
        } else if local == "id" && plain {
            AttrAction::Replace(format!("{prefix}{value}"))
        } else if ID_LIST_ATTRIBUTES.contains(&local.as_str()) && plain {
            match rewrite_id_list(value, prefix, ids) {
                Some(new) => AttrAction::Replace(new),
                None => AttrAction::Remove,
            }
        } else if local == "class" && plain {
            match rewrite_classes(value, prefix) {
                Some(new) => AttrAction::Replace(new),
                None => AttrAction::Keep,
            }
        } else if URL_ATTRIBUTES.contains(&local.as_str()) {
            // Decided on the URL as a URL parser reads it; kept values are
            // written as the XML parser read them (no entity an HTML parser
            // decodes differently).
            match css::url_parser_input(value) {
                None => AttrAction::Remove,
                Some(u) => match u.strip_prefix('#') {
                    Some(rest) if local == "href" => {
                        AttrAction::Replace(format!("#{prefix}{rest}"))
                    }
                    Some(_) => keep_as_read(value),
                    None if css::is_inline_image(&u)
                        || (element == "a" && local == "href" && is_link(&u)) =>
                    {
                        keep_as_read(value)
                    }
                    None => AttrAction::Remove,
                },
            }
        } else if local == "font-family" && plain {
            match css::family_value(fonts, value, quote) {
                Some(new) => AttrAction::Replace(new),
                None => match css::rewrite_value(value, &cx) {
                    css::Rewrite::Unchanged => keep_as_read(value),
                    css::Rewrite::Changed(new) => AttrAction::Replace(new),
                    css::Rewrite::Drop => AttrAction::Remove,
                },
            }
        } else if local == "style" && plain {
            let allowed = is_root.then_some(ROOT_PROPERTIES);
            match css::rewrite_declarations(value, &cx, allowed) {
                Ok(css::Rewrite::Unchanged) => keep_as_read(value),
                Ok(css::Rewrite::Changed(new)) => AttrAction::Replace(new),
                Ok(css::Rewrite::Drop) | Err(_) => AttrAction::Remove,
            }
        } else if URL_PRESENTATION.contains(&local.as_str())
            || value.to_ascii_lowercase().contains("url(")
        {
            match css::rewrite_value(value, &cx) {
                css::Rewrite::Unchanged => keep_as_read(value),
                css::Rewrite::Changed(new) => AttrAction::Replace(new),
                css::Rewrite::Drop => AttrAction::Remove,
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

/// True if the root `<svg>` keeps this attribute (see [`ROOT_ATTRIBUTES`]).
fn root_keeps(attr: &roxmltree::Attribute<'_, '_>) -> bool {
    let local = attr.name().to_ascii_lowercase();
    match attr.namespace() {
        None if local == "role" => {
            ROOT_ROLES.contains(&attr.value().trim().to_ascii_lowercase().as_str())
        }
        None => {
            ROOT_ATTRIBUTES.contains(&local.as_str())
                || ROOT_PROPERTIES.contains(&local.as_str())
                || ROOT_ARIA.contains(&local.as_str())
        }
        Some("http://www.w3.org/XML/1998/namespace") => matches!(local.as_str(), "lang" | "space"),
        Some(_) => false,
    }
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

/// The tokens of a space-separated id list that name ids defined in this
/// SVG, prefixed; the others go (they could only point at the page's own
/// elements). `None`: nothing is left, the attribute goes.
fn rewrite_id_list(value: &str, prefix: &str, ids: &HashSet<String>) -> Option<String> {
    let kept: Vec<String> = value
        .split(|c: char| c.is_ascii_whitespace())
        .filter(|t| !t.is_empty() && ids.contains(*t))
        .map(|t| format!("{prefix}{t}"))
        .collect();
    (!kept.is_empty()).then(|| kept.join(" "))
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

/// Turns a text node holding CDATA into escaped text.
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
        sanitize_svg(svg, 0, &FontSet::new()).unwrap_or_else(|e| panic!("{svg}: {e}"))
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
                r##"<image width="1"/><use/><image href="data:image/png;base64,AA"/><rect fill="none" stroke="url(#sm0-g)"/><a href="https://example.com">k</a>"##
            )
        );
    }

    #[test]
    fn prefixes_ids_and_references() {
        let s = svg(
            r##"<defs><linearGradient id="g0"/><clipPath id='c0'/></defs><path fill="url(#g0)" clip-path="url('#c0')"/><use href="#g0"/><use xlink:href="#c0"/><style>#g0{fill:url(#c0)} #other{}</style>"##,
        );
        let out = sanitize_svg(&s, 3, &FontSet::new()).unwrap();
        assert_eq!(
            out,
            svg(
                r##"<defs><linearGradient id="sm3-g0"/><clipPath id='sm3-c0'/></defs><path fill="url(#sm3-g0)" clip-path="url(#sm3-c0)"/><use href="#sm3-g0"/><use xlink:href="#sm3-c0"/>"##
            )
        );
    }

    #[test]
    fn drops_prolog_comments_pis_and_cdata() {
        let s = format!(
            "<?xml version=\"1.0\"?>\n<!-- c --><svg {NS}><!-- c --><?pi x?><text><![CDATA[a<b&c]]>d</text><style><![CDATA[.a>.b{{}}]]>\n</style></svg>\n"
        );
        assert_eq!(clean(&s), svg("<text>a&lt;b&amp;cd</text>"));
    }

    #[test]
    fn removes_html_breakout_elements() {
        let s = svg(
            r#"<p><style>x</style></p><font face="x">f</font><g><SPAN>s</SPAN></g><foreignObject><div xmlns="http://www.w3.org/1999/xhtml"><p>ok</p></div></foreignObject>"#,
        );
        assert_eq!(clean(&s), svg(r#"<g></g>"#));
    }

    /// Review of #12, finding 4: SMIL could set an `href` or a CSS value
    /// after sanitising (a namespaced `x:attributeName` hid the real one;
    /// escapes and `image-set()` in `to`). All SMIL goes.
    #[test]
    fn removes_all_animation() {
        let s = svg(
            r##"<a><animate attributeName="href" to="https://x"/><set attributeName="xlink:href" to="x"/><set attributeName="onclick" to="x"/><animate attributeName="opacity" to="1"/></a><rect><SET attributeName="fill" to="red"/><animateTransform attributeName="transform" type="rotate" to="9"/><animateMotion path="M0 0"><mpath href="#p"/></animateMotion><discard begin="1s"/><animateColor attributeName="fill" to="red"/></rect>"##,
        );
        assert_eq!(clean(&s), svg(r#"<a></a><rect></rect>"#));
    }

    /// XML reads a literal newline or tab in an attribute as a space; HTML
    /// keeps it, and a newline ends a CSS string. CSS attributes are written
    /// again from the XML parser's view.
    #[test]
    fn css_attributes_are_written_as_checked() {
        let s = svg(
            "<rect style=\"x:'\n;background:url(https://evil.example/n.png);y:'\" fill=\"url(#a)\t\" stroke=\"&#114;ed\" mask=\"none\"/>",
        );
        assert_eq!(
            clean(&s),
            svg(
                "<rect style=\"x:' ;background:url(https://evil.example/n.png);y:'\" fill=\"url(#sm0-a) \" stroke=\"red\" mask=\"none\"/>"
            ),
        );
        // CR from an entity is a newline to CSS: the string ends there.
        let s = svg("<rect style=\"x:'&#13;;background:url(https://evil.example/r.png);y:'\"/>");
        assert_eq!(clean(&s), svg("<rect style=\";background:none;\"/>"));
    }

    /// Review of #12, finding 3: presentation values get the declaration
    /// checks.
    #[test]
    fn presentation_attributes_cannot_load() {
        let s = svg(
            r#"<rect mask="image-set('https://evil.example/d1.png' 1x)" filter="-webkit-image-set('x' 1x)" cursor="image-set('x' 1x), auto" fill="u\72l(https://evil.example/#p)" stroke="url(#g)"/>"#,
        );
        assert_eq!(clean(&s), svg(r#"<rect stroke="url(#sm0-g)"/>"#));
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
                    sanitize_svg(bad, 0, &FontSet::new()),
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
                sanitize_svg(&s, 0, &FontSet::new()),
                Err(GfxError::InvalidSvg { .. })
            ));
        }
        // Self-closing tags don't nest.
        let flat = format!("<svg>{}</svg>", "<g/>".repeat(10_000));
        assert_eq!(clean(&flat), flat);
    }

    /// An island can't affect anything outside its own box (review of #25):
    /// `<style>` applies page-wide and HTML content can restyle, re-parent
    /// or swallow the page, so both go whole.
    #[test]
    fn style_and_foreign_object_go() {
        let s = svg(
            "<style><![CDATA[.zr0-cls-1:hover{fill:red}body{display:none} #total{visibility:hidden}]]></style>\
             <g><style>a{}</style></g><foreignObject width=\"9\"><div xmlns=\"http://www.w3.org/1999/xhtml\"/></foreignObject>\
             <FOREIGNOBJECT/><x:style xmlns:x=\"urn:x\">a{}</x:style><rect/>",
        );
        assert_eq!(clean(&s), svg("<g></g><rect/>"));
    }

    /// `desc` and `title` are HTML integration points: an HTML parser reads
    /// their children as HTML, where `<textarea/>` or `<select/>` swallows
    /// the rest of the page and `<body>` adds attributes to the page's own.
    #[test]
    fn desc_and_title_keep_only_text() {
        let s = svg(
            "<desc id=\"d\">chart &lt;b&gt; <textarea/>a<b>t</b><![CDATA[<x>&]]></desc>\
             <title><select/>b<html lang=\"x\"><body class=\"y\"/></html></title>\
             <g><title>c<math><mi><p>x</p></mi></math></title></g>",
        );
        assert_eq!(
            clean(&s),
            svg(
                "<desc id=\"sm0-d\">chart &lt;b&gt; a&lt;x&gt;&amp;</desc><title>b</title><g><title>c</title></g>"
            )
        );
        // Outside an integration point, <math><mi> is just SVG; a <p> under
        // it would break out of SVG and goes.
        let s = svg("<math><mi><p>x</p></mi></math>");
        assert_eq!(clean(&s), svg("<math><mi></mi></math>"));
    }

    /// The root `<svg>` is the island's box: nothing may move it, put it over
    /// other content, hide it or let it paint outside (review of #25).
    #[test]
    fn root_keeps_only_box_safe_attributes() {
        let s = "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
             width=\"400\" height=\"300\" viewBox=\"0 0 400 300\" version=\"1.1\" baseProfile=\"full\" class=\"marks\" \
             role=\"img\" aria-label=\"c\" xml:lang=\"ja\" fill=\"red\" font-family=\"x\" \
             style=\"position:fixed;top:0;left:0;z-index:9;transform:scale(9);user-select:none;overflow:visible;fill:blue\" \
             transform=\"scale(9)\" overflow=\"visible\" filter=\"url(#f)\" display=\"inline\" visibility=\"hidden\" \
             x=\"1\" xlink:href=\"#a\" clip-path=\"url(#c)\"><rect style=\"position:fixed\" transform=\"scale(2)\"/></svg>";
        assert_eq!(
            clean(s),
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
             width=\"400\" height=\"300\" viewBox=\"0 0 400 300\" version=\"1.1\" baseProfile=\"full\" \
             role=\"img\" aria-label=\"c\" xml:lang=\"ja\" fill=\"red\" font-family=\"x\" \
             style=\";;;;;user-select:none;;fill:blue\"><rect style=\"position:fixed\" transform=\"scale(2)\"/></svg>",
            "inside the root, CSS boxes don't apply to SVG content"
        );
        // The page's CSS can match the root's class, role and ARIA
        // (Tailwind's `fixed inset-0 z-50`): no class, chart roles only.
        let s = "<svg class=\"fixed inset-0 z-50\" role=\"dialog\" aria-modal=\"true\" aria-label=\"x\"><g class=\"fixed\"/></svg>";
        assert_eq!(clean(s), "<svg aria-label=\"x\"><g class=\"fixed\"/></svg>");
    }

    /// An HTML parser compares the root's name as written: `<svg:svg>` is an
    /// unknown HTML element whose content it reads as HTML.
    #[test]
    fn root_must_be_an_unprefixed_svg() {
        for bad in [
            "<svg:svg xmlns:svg=\"http://www.w3.org/2000/svg\"><textarea/></svg:svg>",
            "<x:svg xmlns:x=\"http://www.w3.org/2000/svg\"/>",
        ] {
            assert!(matches!(
                sanitize_svg(bad, 0, &FontSet::new()),
                Err(GfxError::InvalidSvg { .. })
            ));
        }
        assert_eq!(clean("<SVG/>"), "<SVG/>");
    }

    #[test]
    fn id_lists_and_echarts_classes_are_prefixed() {
        let s = svg(
            "<text id=\"t\" aria-labelledby=\"t  other\" aria-describedby=\"d\" aria-owns=\"page-total\">x</text><desc id=\"d\"/>\
             <rect id=\"r\"/>\
             <path class=\"zr0-cls-1 keep zr1-x\"/>",
        );
        assert_eq!(
            sanitize_svg(&s, 4, &FontSet::new()).unwrap(),
            svg(
                "<text id=\"sm4-t\" aria-labelledby=\"sm4-t\" aria-describedby=\"sm4-d\">x</text><desc id=\"sm4-d\"/>\
                 <rect id=\"sm4-r\"/>\
                 <path class=\"sm4-zr0-cls-1 keep sm4-zr1-x\"/>"
            )
        );
        assert!(is_echarts_class("zr0-cls-1"));
        assert!(
            !is_echarts_class("zr-1") && !is_echarts_class("zrx-1") && !is_echarts_class("zr0")
        );
    }

    /// Review of #12 (second round), finding 1: a value that passed as
    /// `data:image/` but that a URL parser reads as a relative URL.
    #[test]
    fn urls_are_read_as_a_url_parser_reads_them() {
        let s = svg(
            "<image href=\"d ata:image/../../r1.png\"/><image href=\"data :image/png,x\"/>\
             <image href=\"da&#x80;ta:image/../../r3.png\"/><image xlink:href=\"da\u{85}ta:image/../r4.png\"/>\
             <filter><feImage href=\"d ata:image/../r6.png\"/></filter>\
             <a href=\"&#x3000;https://example.com/\">l</a><a href=\" https://example.com/\">k</a>\
             <image href=\"&#9;data:image/png;base64,AA&#10;AA\"/><image href=\"#&#x3000;a\"/>",
        );
        assert_eq!(
            clean(&s),
            svg(
                "<image/><image/><image/><image/><filter><feImage/></filter>\
                 <a>l</a><a href=\" https://example.com/\">k</a>\
                 <image href=\"&#9;data:image/png;base64,AA&#10;AA\"/><image href=\"#sm0-\u{3000}a\"/>"
            )
        );
    }

    /// Outlining writes `data-surimono-*` for the conformance checks; an
    /// island can't forge it on any element.
    #[test]
    fn data_surimono_attributes_go() {
        let s = svg("<g data-surimono-text=\"Total: 0\" DATA-SURIMONO-FACE=\"x\" data-x=\"1\"/>");
        assert_eq!(clean(&s), svg("<g data-x=\"1\"/>"));
    }

    /// Chains of <use> fan out exponentially in the printer: a <use> whose
    /// target is or holds a <use> goes.
    #[test]
    fn use_chains_are_cut() {
        let s = svg(
            "<defs><path id=\"p\" d=\"M0 0\"/><g id=\"a\"><use href=\"#p\"/><use href=\"#p\"/></g>\
             <use id=\"u\" href=\"#p\"/></defs>\
             <use href=\"#a\"/><use xlink:href=\" #u\"/><use href=\"#p\"/><g id=\"self\"><use href=\"#self\"/></g>",
        );
        assert_eq!(
            clean(&s),
            svg(
                "<defs><path id=\"sm0-p\" d=\"M0 0\"/><g id=\"sm0-a\"><use href=\"#sm0-p\"/><use href=\"#sm0-p\"/></g>\
                 <use id=\"sm0-u\" href=\"#sm0-p\"/></defs>\
                 <use href=\"#sm0-p\"/><g id=\"sm0-self\"></g>"
            )
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
