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
//! under paged media. The pass has no style engine (a stylo run can't be
//! stopped, and nothing here may run unbounded), so it can't ask what Chrome
//! would compute. It works the other way round: each rewrite has a list of
//! everything it assumes about the page, and every item is either checked on
//! the printer's tree and the CSS's tokens ([`super::rules`]) or the page is
//! left alone. The checks are allow-lists wherever a list is possible: a page
//! is rewritten when all it uses is on the lists of what the pass was
//! measured against in Chromium, not when it merely avoids what is known to
//! break.
//!
//! # What every rewrite assumes
//!
//! 1. **Chrome builds the tree the printer builds.** The pass reads
//!    html5ever's tree as blitz-html sets it up, and Chrome's parser differs
//!    from it: a `<template shadowrootmode>` becomes a shadow root (whose
//!    `:host` and `::slotted()` rules outrank any document rule, and whose
//!    content can follow a slotted node), `<noscript>` holds text, `<select>`
//!    keeps more of its content, and MathML's `annotation-xml` can hold HTML.
//!    Checked: every HTML element is on [`KNOWN_HTML`] or is a custom element,
//!    no `<template>` has a `shadowroot…` attribute, and no `annotation-xml`.
//! 2. **Nothing can look.** No script is left in the page (an island marker
//!    is data): with [`Options::keep_scripts`](crate::Options::keep_scripts)
//!    nothing is rewritten.
//! 3. **All of the page's CSS is in the page and was read to the end**: no
//!    linked style sheet, no `@import`, no at-rule the reader doesn't know
//!    (one that could write declarations into a rule), nothing nested deeper
//!    than a printer may be given. A [`ResourceLoader`](crate::ResourceLoader)
//!    brings linked sheets into the page.
//! 4. **The page is in standards mode**, for border colours and newlines:
//!    what they were measured in. (Sizes were measured in quirks mode too.)
//! 5. **The work is bounded**: [`MAX_STEPS`] steps, counted the same on every
//!    machine, and the deadline everything after the scripts shares. Past
//!    either, the page goes out as it was; only a cancel ends the render.
//!
//! # The sheet two of the rewrites travel in
//!
//! Sizes and border colours are rules in one new `<style>` in `<head>`: in an
//! anonymous cascade layer, after the leading `<meta>`, `<title>` and `<base>`
//! elements, so before every author sheet. The first layer is the lowest: a
//! declaration there loses to every author declaration, layered or not,
//! `!important` or not. It assumes:
//!
//! 6. **No selector can tell that `<head>` has one more element.** `<head>`
//!    and what's in it are never rendered: no rule that may match `<head>`
//!    declares a `display` other than `none`, or `all`. And nothing rendered
//!    is matched by what `<head>` holds: every pseudo-class in the page's
//!    selectors is on the reader's list of those that go by an element's own
//!    state, its ancestors and its siblings (`:has()` is not on it), and
//!    `<head>` already has an element, so `head:empty` says what it said.
//! 7. **A browser decodes the page as before**: it is ASCII, or declares its
//!    encoding ahead of the sheet.
//! 8. **Chrome and the printer read the sheet alike**: it holds no `&`, `<`
//!    or `\`, which blitz-dom decodes and Chrome doesn't.
//!
//! # Size of a block `<svg>`
//!
//! blitz-dom ignores `width` and `height` on an `<svg>` that isn't inline.
//! Chrome maps both attributes of an outermost `<svg>` to presentational
//! hints: declarations just below the author origin. The pass adds a rule per
//! attribute value, `svg[width="640"] { width: 640px }`. It assumes:
//!
//! 9. **Every element the rule matches has that hint, with that value.** Then
//!    the rule only repeats, as the lowest thing in the author origin, what
//!    the hint beneath it says: whatever wins the cascade wins it as before,
//!    and `revert` and `revert-layer` land on the same value. This doesn't
//!    depend on what any property means, so no property is checked. The
//!    selector is namespaced (a MathML element can be named `svg`) and leaves
//!    out an `<svg>` inside another, whose attributes are no hints; the value
//!    is one of SVG's plain lengths (digits, a fraction, an absolute unit or
//!    none); with assumption 1 the element is in the document's own tree.
//! 10. **The attribute isn't animated**: no SMIL element in the page.
//!
//! An inline `style` would outrank author rules the attribute loses to, which
//! is why this isn't one.
//!
//! # Colour of a border on one side
//!
//! fulgur paints all four sides in `border-top-color`. For each rule with a
//! `border-bottom`, `-left` or `-right` that paints, the pass adds
//! `border-top-color` with that side's colour, under the same selector. The
//! copy can only take effect where no author declaration sets the top
//! border's colour. It assumes:
//!
//! 11. **Where it takes effect, no top border paints.** Every border style on
//!     the page comes from a shorthand that sets the colour too, so a top
//!     border that paints has an author colour above the layer. Checked: every
//!     property with `border` in its name is on [`BORDER_PROPERTIES`] (no
//!     style longhand, no logical property, no `-webkit-border-…` alias), and
//!     nothing declares `all`.
//! 12. **The element has no border of the user agent's or of an attribute's.**
//!     The selector can only match elements on [`BORDERLESS`] whose attributes
//!     are all on [`PLAIN_ATTRIBUTES`] (so no `popover`), and no table part
//!     has an attribute off it (so no `border`, `rules` or `frame`).
//! 13. **Nothing reads the colour another way**: no border declaration says
//!     `inherit` or `revert` (the user agent's table rows inherit theirs), and
//!     nothing says `revert-layer`.
//! 14. **The colour is the one that prints.** All one-sided borders of the
//!     page are plain rules of sheets that always apply, unconditional, with
//!     a literal colour and no `!important`, so the copies cascade among
//!     themselves as the originals do. Otherwise none is copied.
//!
//! # Newline ending a `<pre>`
//!
//! parley gives a layout ending in a newline one more line, and blitz-dom
//! keeps it. Chrome ends the last line at that newline and starts no other,
//! so the newline goes. No selector sees a text node, and the node isn't
//! emptied. It assumes:
//!
//! 15. **The newline ends a line that has content**: it follows a visible
//!     character in the same text node. After an empty line it *is* the line.
//! 16. **Nothing follows it in the `<pre>`**: the text is the last thing in
//!     it, through elements on [`PLAIN_INLINE`] only (`<q>` adds a quote),
//!     and no `::after` (or pseudo-element the reader doesn't know) with
//!     `content` may match an element on that path.
//! 17. **The `<pre>` is a block container, and the path is made of inline
//!     boxes laid out left to right.** No rule that may match the `<pre>`
//!     declares a `display` off [`BLOCK_DISPLAYS`], none that may match a
//!     path element one off [`PATH_DISPLAYS`]. Chrome lays an inline's end
//!     edge out after the break once bidi is involved, so: the `<pre>` and
//!     the path have only attributes on [`PATH_ATTRIBUTES`] (no `dir`), no
//!     ancestor has a `dir` other than `ltr`, the page has no `<bdi>` or
//!     `<bdo>`, and the `<pre>` holds no right-to-left character or bidi
//!     control.
//! 18. **The page's CSS is CSS the newline was measured under.** Every
//!     property declared anywhere in the page (an inherited one reaches the
//!     `<pre>` from anywhere) is on [`NEWLINE_PROPERTIES`]: each was printed
//!     by Chromium 153 on the `<pre>`, on its inline elements and on `<body>`,
//!     with the newline and without, pixel for pixel the same. `direction`,
//!     `unicode-bidi`, `writing-mode`, `white-space-collapse` and the rest
//!     aren't on it; `white-space` is, with the six values measured.

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
/// Most steps the pass takes: nodes walked, CSS tokens read, and elements
/// held against rules. A page of a few megabytes takes under a million.
const MAX_STEPS: u64 = 1 << 24;

/// The HTML elements the pass knows: Chrome's parser and html5ever's build the
/// same tree from them. Not here, among others: `select`, `option`,
/// `optgroup` and `datalist` (Chrome keeps more of a `<select>`), `noscript`,
/// `rb` and `rtc`, `slot`, frames, and the raw-text relics. A custom element
/// (a hyphen in its name) is known too: no parser treats one specially.
const KNOWN_HTML: &[&str] = &[
    "a",
    "abbr",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "bdi",
    "bdo",
    "big",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "dd",
    "del",
    "details",
    "dfn",
    "dialog",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "font",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "menu",
    "meta",
    "meter",
    "nav",
    "nobr",
    "object",
    "ol",
    "output",
    "p",
    "param",
    "picture",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "search",
    "section",
    "small",
    "source",
    "span",
    "strike",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "tt",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

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

/// The parts of a table: a `border`, `rules` or `frame` on one gives others a
/// border.
const TABLE_PARTS: &[&str] = &[
    "table", "caption", "colgroup", "col", "thead", "tbody", "tfoot", "tr", "td", "th",
];

/// Attributes that give no element a border, of its own or of the user
/// agent's: with `data-…`, `aria-…` and the names only a framework writes
/// (see [`plain_attribute`]), all an element may have for a border colour to
/// be copied onto it.
const PLAIN_ATTRIBUTES: &[&str] = &[
    "abbr",
    "align",
    "bgcolor",
    "cellpadding",
    "cellspacing",
    "cite",
    "class",
    "colspan",
    "datetime",
    "dir",
    "download",
    "for",
    "headers",
    "height",
    "hidden",
    "href",
    "id",
    "lang",
    "name",
    "nowrap",
    "open",
    "rel",
    "reversed",
    "role",
    "rowspan",
    "scope",
    "span",
    "start",
    "style",
    "summary",
    "tabindex",
    "target",
    "title",
    "translate",
    "type",
    "valign",
    "value",
    "width",
];

/// All a `<pre>` whose newline goes, and the elements its last text is in, may
/// have: nothing that sets a direction, or anything else, as a hint.
const PATH_ATTRIBUTES: &[&str] = &[
    "class", "href", "id", "name", "rel", "role", "style", "tabindex", "target", "title",
];

/// Inline HTML elements a `<pre>`'s last text may be in: they add nothing
/// after their content and hold only what's in the tree.
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

/// `display` values an element between a `<pre>` and its last text may have.
const PATH_DISPLAYS: &[&str] = &["inline", "inline-block", "block", "contents", "none"];

/// `white-space` values the newline was measured under.
const WHITE_SPACES: &[&str] = &[
    "normal",
    "pre",
    "pre-wrap",
    "pre-line",
    "break-spaces",
    "nowrap",
];

/// The properties with `border` in their name under which a border colour may
/// be copied: none sets a border's style without its colour, and none is a
/// logical or legacy name for one.
const BORDER_PROPERTIES: &[&str] = &[
    "border",
    "border-bottom",
    "border-bottom-color",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-bottom-width",
    "border-collapse",
    "border-color",
    "border-image",
    "border-left",
    "border-left-color",
    "border-left-width",
    "border-radius",
    "border-right",
    "border-right-color",
    "border-right-width",
    "border-spacing",
    "border-top",
    "border-top-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-top-width",
    "border-width",
];

/// The properties a `<pre>`'s last newline was measured under (assumption
/// 18), sorted. Each was printed by Chromium 153 with a sample value on the
/// `<pre>`, on the inline elements its last text is in (which had padding, a
/// border and a margin), on `<body>` and on all of them, with the newline
/// and without: the same pixels. `display` and `white-space` are here for
/// the values on their own lists. Not here, among others: `direction`,
/// `unicode-bidi`, `writing-mode`, `white-space-collapse`, `text-wrap-mode`,
/// `line-clamp`, and anything prefixed that isn't listed.
const NEWLINE_PROPERTIES: &[&str] = &[
    "-moz-osx-font-smoothing",
    "-webkit-appearance",
    "-webkit-border-radius",
    "-webkit-box-decoration-break",
    "-webkit-box-shadow",
    "-webkit-box-sizing",
    "-webkit-font-smoothing",
    "-webkit-overflow-scrolling",
    "-webkit-print-color-adjust",
    "-webkit-tap-highlight-color",
    "-webkit-text-size-adjust",
    "-webkit-transform",
    "-webkit-transition",
    "-webkit-user-select",
    "accent-color",
    "align-content",
    "align-items",
    "align-self",
    "animation",
    "appearance",
    "aspect-ratio",
    "background",
    "background-attachment",
    "background-clip",
    "background-color",
    "background-image",
    "background-origin",
    "background-position",
    "background-repeat",
    "background-size",
    "block-size",
    "border",
    "border-block",
    "border-block-end",
    "border-block-start",
    "border-bottom",
    "border-bottom-color",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-bottom-style",
    "border-bottom-width",
    "border-collapse",
    "border-color",
    "border-image",
    "border-inline",
    "border-inline-end",
    "border-inline-start",
    "border-left",
    "border-left-color",
    "border-left-style",
    "border-left-width",
    "border-radius",
    "border-right",
    "border-right-color",
    "border-right-style",
    "border-right-width",
    "border-spacing",
    "border-style",
    "border-top",
    "border-top-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-top-style",
    "border-top-width",
    "border-width",
    "bottom",
    "box-decoration-break",
    "box-shadow",
    "box-sizing",
    "break-after",
    "break-before",
    "break-inside",
    "caption-side",
    "caret-color",
    "clear",
    "clip-path",
    "color",
    "color-scheme",
    "column-count",
    "column-gap",
    "column-rule",
    "column-width",
    "columns",
    "contain",
    "content",
    "content-visibility",
    "counter-increment",
    "counter-reset",
    "cursor",
    "display",
    "empty-cells",
    "filter",
    "flex",
    "flex-basis",
    "flex-direction",
    "flex-grow",
    "flex-shrink",
    "flex-wrap",
    "float",
    "font",
    "font-family",
    "font-feature-settings",
    "font-kerning",
    "font-optical-sizing",
    "font-size",
    "font-size-adjust",
    "font-stretch",
    "font-style",
    "font-synthesis",
    "font-variant",
    "font-variant-ligatures",
    "font-variant-numeric",
    "font-variation-settings",
    "font-weight",
    "gap",
    "grid-area",
    "grid-column",
    "grid-template-columns",
    "height",
    "hyphens",
    "image-rendering",
    "inline-size",
    "inset",
    "isolation",
    "justify-content",
    "justify-items",
    "justify-self",
    "left",
    "letter-spacing",
    "line-break",
    "line-height",
    "list-style",
    "list-style-position",
    "list-style-type",
    "margin",
    "margin-block",
    "margin-block-end",
    "margin-block-start",
    "margin-bottom",
    "margin-inline",
    "margin-inline-end",
    "margin-inline-start",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-block-size",
    "max-height",
    "max-inline-size",
    "max-width",
    "min-block-size",
    "min-height",
    "min-inline-size",
    "min-width",
    "mix-blend-mode",
    "object-fit",
    "object-position",
    "opacity",
    "order",
    "orphans",
    "outline",
    "outline-color",
    "outline-offset",
    "outline-style",
    "outline-width",
    "overflow",
    "overflow-wrap",
    "overflow-x",
    "overflow-y",
    "padding",
    "padding-block",
    "padding-block-end",
    "padding-block-start",
    "padding-bottom",
    "padding-inline",
    "padding-inline-end",
    "padding-inline-start",
    "padding-left",
    "padding-right",
    "padding-top",
    "page-break-after",
    "page-break-before",
    "page-break-inside",
    "place-items",
    "pointer-events",
    "position",
    "print-color-adjust",
    "quotes",
    "resize",
    "right",
    "scroll-behavior",
    "tab-size",
    "table-layout",
    "text-align",
    "text-align-last",
    "text-decoration",
    "text-decoration-color",
    "text-decoration-line",
    "text-decoration-skip-ink",
    "text-decoration-style",
    "text-decoration-thickness",
    "text-emphasis",
    "text-indent",
    "text-overflow",
    "text-rendering",
    "text-shadow",
    "text-size-adjust",
    "text-spacing-trim",
    "text-transform",
    "text-underline-offset",
    "text-underline-position",
    "text-wrap",
    "top",
    "transform",
    "transform-origin",
    "transition",
    "user-select",
    "vertical-align",
    "visibility",
    "white-space",
    "widows",
    "width",
    "will-change",
    "word-break",
    "word-spacing",
    "word-wrap",
    "z-index",
    "zoom",
];

/// Units an SVG length and a CSS length share, with the same meaning.
const ABSOLUTE_UNITS: &[&str] = &["px", "in", "cm", "mm", "pt", "pc"];

/// Why the pass stops before its end.
enum Stop {
    /// It took [`MAX_STEPS`]: the page goes out as it was.
    Budget,
    /// The gate: a cancel, or the deadline.
    Gate(Error),
}

/// What the pass has cost so far, and the gate it runs under.
struct Work<'g> {
    steps: u64,
    gate: &'g Gate<'g>,
}

impl Work<'_> {
    /// Counts `n` steps. Looks at the gate every few thousand.
    fn step(&mut self, n: u64) -> Result<(), Stop> {
        let before = self.steps;
        self.steps = self.steps.saturating_add(n);
        if self.steps > MAX_STEPS {
            return Err(Stop::Budget);
        }
        if before >> 12 != self.steps >> 12 || before == 0 {
            self.gate.check().map_err(Stop::Gate)?;
        }
        Ok(())
    }
}

/// A `<pre>` whose text ends in a newline Chrome doesn't render, as far as
/// the tree says.
struct Pre {
    /// The `<pre>`, then the elements its last text is in, outermost first.
    path: Vec<usize>,
    /// That text node.
    text: usize,
}

/// What one walk over the tree finds.
#[derive(Default)]
struct Facts {
    head: Option<usize>,
    /// A `<link>` to a style sheet: CSS that isn't in the page.
    links: bool,
    smil: bool,
    /// A table part has an attribute that may give its cells a border.
    table_attributes: bool,
    /// `<style>` elements, in any namespace and in templates: a printer reads
    /// them all, and Chrome some.
    styles: Vec<usize>,
    /// Elements with a `style` attribute.
    styled: Vec<usize>,
    /// Elements that may have a border of the user agent's, or an attribute's.
    bordered: Vec<usize>,
    /// `width` and `height` values of outermost `<svg>` elements, each once,
    /// in document order.
    sizes: Vec<(&'static str, String)>,
    pres: Vec<Pre>,
}

fn is_svg(e: &Element) -> bool {
    e.name.ns == ns!(svg)
}

fn html_named(e: &Element, names: &[&str]) -> bool {
    e.name.ns == ns!(html) && names.contains(&e.local())
}

/// Whether an attribute is one that styles nothing: on `names`, or a name no
/// browser gives a meaning (`data-…`, `aria-…`, and what frameworks write:
/// `x-data`, `v-if`, `@click`, `:class`, `hx-get`). HTML's own hyphenated
/// attributes are the two listed.
fn plain_attribute(name: &str, names: &[&str]) -> bool {
    names.contains(&name)
        || name.contains([':', '@', '.'])
        || (name.contains('-') && !matches!(name, "http-equiv" | "accept-charset"))
}

fn plain_attributes(e: &Element, names: &[&str]) -> bool {
    e.attrs
        .iter()
        .all(|a| a.name.ns == ns!() && plain_attribute(a.name.local.as_ref(), names))
}

/// Whether `c` is laid out right to left, or steers what is: Hebrew, Arabic
/// and the other right-to-left scripts, and the bidi controls.
fn right_to_left(c: char) -> bool {
    matches!(
        c,
        '\u{590}'..='\u{8ff}' | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}' | '\u{fb1d}'..='\u{fdff}' | '\u{fe70}'..='\u{feff}'
            | '\u{10800}'..='\u{10fff}' | '\u{1e800}'..='\u{1efff}'
    )
}

/// The newline ending `pre`, if the tree has nothing against dropping it:
/// assumptions 15, 16 and 17, as far as they are the tree's.
fn pre_newline(tree: &Tree, pre: usize, work: &mut Work<'_>) -> Result<Option<Pre>, Stop> {
    let mut path = vec![pre];
    let mut at = pre;
    let text = loop {
        work.step(1)?;
        if !tree
            .element(at)
            .is_some_and(|e| plain_attributes(e, PATH_ATTRIBUTES))
        {
            return Ok(None);
        }
        let last = tree.nodes[at]
            .children
            .iter()
            .rev()
            .copied()
            .find(|&c| !matches!(tree.nodes[c].data, Data::Comment));
        let Some(last) = last else {
            return Ok(None);
        };
        match &tree.nodes[last].data {
            Data::Text(_) => break last,
            Data::Element(e) if html_named(e, PLAIN_INLINE) => {
                path.push(last);
                at = last;
            }
            _ => return Ok(None),
        }
    };
    let Data::Text(content) = &tree.nodes[text].data else {
        return Ok(None);
    };
    let mut end = content.chars().rev();
    if end.next() != Some('\n') || !end.next().is_some_and(visible) {
        return Ok(None);
    }
    // Everything the `<pre>` holds is in its lines.
    let mut stack = vec![pre];
    while let Some(id) = stack.pop() {
        let node = &tree.nodes[id];
        if let Data::Text(t) = &node.data {
            work.step(1 + t.len() as u64 / 64)?;
            if t.chars().any(right_to_left) {
                return Ok(None);
            }
        }
        work.step(1)?;
        stack.extend(node.children.iter().copied());
    }
    Ok(Some(Pre { path, text }))
}

/// What a node is in, for the walk.
#[derive(Clone, Copy, Default)]
struct Around {
    /// An HTML `<template>`: nothing here is rendered by Chrome.
    inert: bool,
    /// An `<svg>`.
    svg: bool,
    /// An element with a `dir` that isn't `ltr`.
    directed: bool,
}

/// Walks the tree. `None` if it holds something the pass doesn't know
/// (assumptions 1 and 2): then nothing is rewritten.
fn facts(tree: &Tree, work: &mut Work<'_>) -> Result<Option<Facts>, Stop> {
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
    // `<bdi>` and `<bdo>` set a direction with no attribute.
    let mut bidi = false;
    let mut pres = Vec::new();
    let mut stack = vec![(0, Around::default())];
    while let Some((id, around)) = stack.pop() {
        work.step(1)?;
        let node = &tree.nodes[id];
        let mut inside = around;
        if let Some(e) = tree.element(id) {
            let local = e.local();
            let html_element = e.name.ns == ns!(html);
            work.step(e.attrs.len() as u64)?;
            let known = if html_element {
                KNOWN_HTML.contains(&local) || local.contains('-')
            } else {
                !local.eq_ignore_ascii_case("annotation-xml")
            };
            let shadow = e.is_html("template")
                && e.attrs
                    .iter()
                    .any(|a| a.name.local.starts_with("shadowroot"));
            let script = local.eq_ignore_ascii_case("script")
                && !(e.is_html("script") && e.attr("type").is_some_and(is_island_type));
            if !known || shadow || script {
                return Ok(None);
            }
            inside.inert |= e.is_html("template");
            inside.directed |= e
                .attr("dir")
                .is_some_and(|d| !d.eq_ignore_ascii_case("ltr"));
            bidi |= html_named(e, &["bdi", "bdo"]);
            if local.eq_ignore_ascii_case("link")
                && e.attr("rel")
                    .is_some_and(|r| r.to_ascii_lowercase().contains("stylesheet"))
            {
                facts.links = true;
            }
            facts.smil |= SMIL.iter().any(|s| local.eq_ignore_ascii_case(s));
            if e.attr("style").is_some() {
                facts.styled.push(id);
            }
            if local == "style" {
                facts.styles.push(id);
            }
            let plain = plain_attributes(e, PLAIN_ATTRIBUTES);
            facts.table_attributes |= html_named(e, TABLE_PARTS) && !plain;
            let borderless =
                html_element && plain && (BORDERLESS.contains(&local) || local.contains('-'));
            if !borderless {
                facts.bordered.push(id);
            }
            if e.is_html("pre") && !around.inert && !inside.directed {
                pres.push(id);
            }
            let svg = is_svg(e) && local == "svg";
            // One the added rule can match: in no other `<svg>`, and rendered.
            if svg && !around.svg && !around.inert {
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
            inside.svg |= svg;
        }
        stack.extend(node.children.iter().rev().map(|&c| (c, inside)));
    }
    if !bidi {
        for pre in pres {
            facts.pres.extend(pre_newline(tree, pre, work)?);
        }
    }
    Ok(Some(facts))
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
    /// A hash of hex digits, or `rgb()` or `hsl()` of plain numbers, written
    /// from its own tokens.
    Colour(String),
    /// A number or a length.
    Size,
    /// `!`, as in `!important`.
    Bang,
    Other,
}

/// `#` and the 3, 4, 6 or 8 hex digits of a colour, from the token's value.
fn hash_colour(digits: &str) -> Word {
    if matches!(digits.len(), 3 | 4 | 6 | 8) && digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        Word::Colour(format!("#{digits}"))
    } else {
        Word::Other
    }
}

/// The words of a declaration's value.
fn words(value: &str) -> Vec<Word> {
    let mut input = ParserInput::new(value);
    let mut p = Parser::new(&mut input);
    let mut all = Vec::new();
    while let Ok(token) = p.next() {
        let word = match token.clone() {
            Token::Ident(name) => Word::Ident(name.to_ascii_lowercase()),
            Token::Hash(digits) | Token::IDHash(digits) => hash_colour(&digits),
            Token::Number { .. } | Token::Dimension { .. } => Word::Size,
            Token::Delim('!') => Word::Bang,
            Token::Function(name)
                if ["rgb", "rgba", "hsl", "hsla"]
                    .iter()
                    .any(|f| name.eq_ignore_ascii_case(f)) =>
            {
                let name = name.to_ascii_lowercase();
                // Only numbers, percentages, commas and a slash, each written
                // back as its own text.
                let inner: Result<Option<String>, cssparser::ParseError<'_, ()>> = p
                    .parse_nested_block(|p| {
                        let mut text = String::new();
                        loop {
                            let start = p.position();
                            let Ok(token) = p.next() else {
                                break;
                            };
                            let plain =
                                matches!(token, Token::Number { .. } | Token::Percentage { .. });
                            let mark = match token {
                                Token::Comma => Some(","),
                                Token::Delim('/') => Some("/"),
                                _ => None,
                            };
                            let own = p.slice_from(start).trim();
                            let digits = own
                                .bytes()
                                .all(|b| b.is_ascii_digit() || b"+-.%".contains(&b));
                            match mark {
                                Some(mark) => text.push_str(mark),
                                None if plain && digits && !own.is_empty() => {
                                    if text.ends_with(|c: char| c.is_ascii_digit() || c == '%') {
                                        text.push(' ');
                                    }
                                    text.push_str(own);
                                }
                                None => return Ok(None),
                            }
                        }
                        Ok(Some(text))
                    });
                match inner {
                    Ok(Some(text)) => Word::Colour(format!("{name}({text})")),
                    _ => Word::Other,
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

/// What one look at every declaration of the page finds.
#[derive(Default)]
struct Sheet {
    /// Rules that declare `display` or `all`.
    display: Vec<usize>,
    /// Rules that declare `content` or `all` for an `::after`, or for a
    /// pseudo-element the reader doesn't know.
    after: Vec<usize>,
    /// Every property is on [`NEWLINE_PROPERTIES`], `white-space` with a value
    /// on [`WHITE_SPACES`]: assumption 18.
    measured: bool,
    /// Every property with `border` in its name is on [`BORDER_PROPERTIES`],
    /// none says `inherit` or `revert`, and nothing declares `all`:
    /// assumptions 11 and 13.
    borders: bool,
}

fn sheet(rules: &Rules, work: &mut Work<'_>) -> Result<Sheet, Stop> {
    let mut sheet = Sheet {
        measured: true,
        borders: !rules.flags.revert_layer,
        ..Sheet::default()
    };
    for (index, rule) in rules.rules.iter().enumerate() {
        work.step(1 + rule.decls.len() as u64)?;
        let (mut display, mut content) = (false, false);
        for decl in &rule.decls {
            let name = decl.name.as_str();
            display |= matches!(name, "display" | "all");
            content |= matches!(name, "content" | "all");
            let custom = name.starts_with("--");
            if !custom && NEWLINE_PROPERTIES.binary_search(&name).is_err() {
                sheet.measured = false;
            }
            if name == "white-space"
                && !one_ident(rules.value(rule, decl))
                    .is_some_and(|v| WHITE_SPACES.contains(&v.as_str()))
            {
                sheet.measured = false;
            }
            if name == "all" {
                sheet.borders = false;
            }
            if !custom && name.contains("border") {
                let handed_on = || {
                    words(rules.value(rule, decl)).iter().any(|w| {
                        matches!(w, Word::Ident(v) if matches!(v.as_str(), "inherit" | "revert"))
                    })
                };
                if !BORDER_PROPERTIES.contains(&name) || handed_on() {
                    sheet.borders = false;
                }
            }
        }
        if display {
            sheet.display.push(index);
        }
        if content && rule.subjects.iter().any(|s| s.after) {
            sheet.after.push(index);
        }
    }
    Ok(sheet)
}

/// Where the new `<style>` goes, if the page's CSS can't see one: `<head>` and
/// the index among its children. Assumptions 6 and 7.
fn carrier(
    tree: &Tree,
    facts: &Facts,
    rules: &Rules,
    sheet: &Sheet,
    ascii: bool,
) -> Option<(usize, usize)> {
    let head = facts.head?;
    let element = tree.element(head)?;
    if rules.flags.has || rules.flags.unknown_pseudo_class {
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
            let content_type = e
                .attr("http-equiv")
                .is_some_and(|h| h.eq_ignore_ascii_case("content-type"))
                && e.attr("content")
                    .is_some_and(|c| c.to_ascii_lowercase().contains("charset"));
            declared |= e.attr("charset").is_some() || content_type;
        } else if !e.is_html("title") && !e.is_html("base") {
            at = i;
            break;
        }
    }
    if !ascii && !declared {
        return None;
    }
    let shown = sheet.display.iter().any(|&index| {
        let rule = &rules.rules[index];
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
        && matches!(name.as_str(), "initial" | "unset")
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
        [Word::Ident(name)] if matches!(name.as_str(), "initial" | "unset") => {
            Side::Colour("currentcolor".to_owned())
        }
        [Word::Ident(name)] if colour_name(name) => Side::Colour(name.clone()),
        _ => Side::Unknown,
    }
}

/// Whether a selector can be written into the added sheet as it is: nothing
/// that means something else there (a namespace prefix, a nesting selector),
/// or that a printer or a parser would read differently from Chrome
/// (assumption 8), and no comment.
fn copyable(selector: &str) -> bool {
    !selector.is_empty()
        && !selector.contains("/*")
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
/// none if the page's borders can't all be read: assumptions 11 to 14.
fn border_colours(
    tree: &Tree,
    facts: &Facts,
    rules: &Rules,
    sheet: &Sheet,
    work: &mut Work<'_>,
) -> Result<Vec<String>, Stop> {
    const SIDES: [&str; 3] = ["border-bottom", "border-left", "border-right"];
    const TOP: [&str; 4] = ["border", "border-top", "border-color", "border-top-color"];
    let none = Ok(Vec::new());
    if !sheet.borders || facts.table_attributes {
        return none;
    }
    let mut copies = Vec::new();
    let mut bytes = 0;
    for rule in &rules.rules {
        work.step(1)?;
        let mut colours = Vec::new();
        for decl in &rule.decls {
            let name = decl.name.as_str();
            let value = rules.value(rule, decl);
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
                // One that may not apply when its rule does can't be copied.
                Side::Colour(_) if decl.conditional => return none,
                Side::Colour(colour) => colours.push(colour),
            }
        }
        if colours.is_empty() {
            continue;
        }
        if !mirrors(tree, facts, rules, rule, work)? {
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
/// sheet and can't match a pseudo-element or an element that may have a
/// border of the user agent's or an attribute's.
fn mirrors(
    tree: &Tree,
    facts: &Facts,
    rules: &Rules,
    rule: &Rule,
    work: &mut Work<'_>,
) -> Result<bool, Stop> {
    let Origin::Sheet(node) = rule.origin else {
        return Ok(false);
    };
    let readable = rule.plain
        && applies_everywhere(tree, node)
        && on_one_line(rules.selector(rule)).is_some_and(|s| copyable(&s))
        && !rule.subjects.is_empty()
        && rule.subjects.iter().all(|s| !s.any && !s.pseudo_element);
    if !readable {
        return Ok(false);
    }
    for &id in &facts.bordered {
        work.step(1)?;
        if tree
            .element(id)
            .is_some_and(|e| rule.subjects.iter().any(|s| s.may_match(e)))
        {
            return Ok(false);
        }
    }
    Ok(true)
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
/// `<pre>`, where the page's CSS has nothing against it. Assumptions 16 to 18,
/// as far as they are the CSS's.
fn trailing_newlines(
    tree: &Tree,
    facts: &Facts,
    rules: &Rules,
    sheet: &Sheet,
    work: &mut Work<'_>,
) -> Result<Vec<usize>, Stop> {
    let mut found = Vec::new();
    if !sheet.measured {
        return Ok(found);
    }
    'pre: for pre in &facts.pres {
        for &index in &sheet.display {
            let rule = &rules.rules[index];
            for (depth, &id) in pre.path.iter().enumerate() {
                work.step(1)?;
                let Some(element) = tree.element(id) else {
                    continue 'pre;
                };
                if !rule.may_match(id, element) {
                    continue;
                }
                let allowed = if depth == 0 {
                    BLOCK_DISPLAYS
                } else {
                    PATH_DISPLAYS
                };
                let other = rule.decls.iter().any(|d| {
                    d.name == "all"
                        || (d.name == "display"
                            && !one_ident(rules.value(rule, d))
                                .is_some_and(|v| allowed.contains(&v.as_str())))
                });
                if other {
                    continue 'pre;
                }
            }
        }
        for &index in &sheet.after {
            let rule = &rules.rules[index];
            if !matches!(rule.origin, Origin::Sheet(_)) {
                continue;
            }
            for &id in &pre.path {
                work.step(1)?;
                let behind =
                    |s: &Subject| s.after && tree.element(id).is_some_and(|e| s.may_match(e));
                if rule.subjects.iter().any(behind) {
                    continue 'pre;
                }
            }
        }
        found.push(pre.text);
    }
    Ok(found)
}

/// Rewrites `tree`, the printer's parse of a page that has passed every
/// screen, into its printer-safe form, and says whether anything changed.
/// `ascii` says whether the page's text is all ASCII. See the module docs.
///
/// `Ok(false)` also when the pass gave up at [`MAX_STEPS`]: the tree may then
/// be half rewritten, and isn't to be used. The only errors are the gate's.
pub(super) fn rewrite(tree: &mut Tree, ascii: bool, gate: &Gate<'_>) -> Result<bool, Error> {
    let mut work = Work { steps: 0, gate };
    match rewrite_within(tree, ascii, &mut work) {
        Ok(changed) => Ok(changed),
        Err(Stop::Budget) => Ok(false),
        Err(Stop::Gate(e)) => Err(e),
    }
}

fn rewrite_within(tree: &mut Tree, ascii: bool, work: &mut Work<'_>) -> Result<bool, Stop> {
    let Some(facts) = facts(tree, work)? else {
        return Ok(false);
    };
    let mut texts = Vec::with_capacity(facts.styles.len());
    let mut borders = false;
    for &id in &facts.styles {
        let css = tree.text_of(id);
        work.step(1 + css.len() as u64 / 64)?;
        if !borders {
            let lower = css.to_ascii_lowercase();
            borders = ["border-bottom", "border-left", "border-right"]
                .iter()
                .any(|side| lower.contains(side));
        }
        texts.push((id, css));
    }
    if facts.sizes.is_empty() && facts.pres.is_empty() && !borders {
        return Ok(false);
    }
    let mut rules = Rules::default();
    let mut read = 0;
    for (id, css) in texts {
        rules.read_sheet(id, css, work.gate).map_err(Stop::Gate)?;
        work.step((rules.tokens() - read) as u64)?;
        read = rules.tokens();
    }
    for &id in &facts.styled {
        let Some(e) = tree.element(id) else {
            continue;
        };
        for a in e.attrs.iter().filter(|a| a.name.local.as_ref() == "style") {
            rules
                .read_attribute(id, a.value.to_string(), work.gate)
                .map_err(Stop::Gate)?;
            work.step(1 + (rules.tokens() - read) as u64)?;
            read = rules.tokens();
        }
    }
    // Assumption 3: CSS that isn't here, wasn't read to its end, or holds an
    // at-rule the reader doesn't know could say anything.
    let flags = rules.flags;
    if rules.unread || flags.import || flags.unknown_at_rule || facts.links {
        return Ok(false);
    }
    let sheet = sheet(&rules, work)?;
    // Assumption 4.
    let standards = !tree.quirks();
    let mut changed = false;
    let newlines = if standards {
        trailing_newlines(tree, &facts, &rules, &sheet, work)?
    } else {
        Vec::new()
    };
    for text in newlines {
        if let Data::Text(content) = &mut tree.nodes[text].data {
            content.pop();
            changed = true;
        }
    }
    if let Some((head, at)) = carrier(tree, &facts, &rules, &sheet, ascii) {
        let sizes = svg_sizes(&facts);
        let colours = if borders && standards {
            border_colours(tree, &facts, &rules, &sheet, work)?
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
            // A pseudo-class or an at-rule the reader doesn't know.
            ("a:popover-open { color: red }", ""),
            ("div:host-context(p) { color: red }", ""),
            (".x { @apply --mixin; }", ""),
            ("@position-try --a { top: 0 }", ""),
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
                "blockquote{border-top-color:rgb(0,0,0/50%)}",
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
            ("", "<img class=\"row\" border=\"2\" src=\"a.png\">"),
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
            // An `<img border>` has a border of its own; the rule can't match it.
            let body = format!("{ROWS}<hr><input><img border=\"2\" src=\"a.png\">");
            let html = page(&format!("{ROW}\n{css}"), &body);
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

    const LIMITS: super::super::TreeLimits = super::super::TreeLimits {
        max_nodes: 1 << 20,
        max_depth: 512,
        max_attributes: 4_096,
    };

    /// The reviewer's blocker 5. The pass is an improvement: out of time or
    /// out of steps, the page goes out as it was, and only a cancel ends the
    /// render.
    #[test]
    fn a_pass_that_doesnt_fit_gives_the_page_as_it_was() {
        let html = page("svg { display: block }", SVG);
        let settled = plain(&html);
        let stop = AtomicBool::new(false);
        let wide = Gate::new(&stop, Duration::from_secs(600));
        let screen = |tree: &mut Tree| tree.to_html(WIDE, &wide);
        let run =
            |gate: &Gate<'_>| super::super::printer_safe(&settled, LIMITS, WIDE, gate, screen);
        assert!(matches!(run(&wide), Ok(Some(_))));
        let late = Gate::new(&stop, Duration::ZERO);
        assert!(matches!(run(&late), Ok(None)), "the deadline: plain");
        let stopped = AtomicBool::new(true);
        let cancelled = Gate::new(&stopped, Duration::from_secs(600));
        assert!(matches!(run(&cancelled), Err(Error::Cancelled)));
    }

    /// The reviewer's page: 20,000 `<pre>` and 20,000 rules, which took the
    /// pass 7.6 s. Rules that declare no `display`, `all` or `content` cost a
    /// `<pre>` nothing, so this is linear, and every newline goes.
    #[test]
    fn many_pres_and_many_rules_are_linear() {
        let (mut css, mut body) = (String::new(), String::new());
        for i in 0..20_000 {
            css.push_str(&format!(".c{i}{{color:red}}"));
            body.push_str("<pre>a\n</pre>");
        }
        let html = page(&css, &body);
        let settled = plain(&html);
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::from_secs(600));
        let mut tree = Tree::parse(&settled, LIMITS, &gate).unwrap_or_else(|e| panic!("{e}"));
        let mut work = Work {
            steps: 0,
            gate: &gate,
        };
        assert!(matches!(
            rewrite_within(&mut tree, true, &mut work),
            Ok(true)
        ));
        // About six steps a `<pre>` and a few a rule: nowhere near a product.
        assert!(work.steps < 1_000_000, "{}", work.steps);
        let out = tree.to_html(WIDE, &gate).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out.matches("<pre>a</pre>").count(), 20_000);
    }

    /// Where every `<pre>` does have to be held against every rule, the steps
    /// run out, the same on every machine, and the page is the plain one.
    #[test]
    fn work_past_the_step_budget_is_given_up() {
        let (mut css, mut body) = (String::new(), String::new());
        for i in 0..6_000 {
            css.push_str(&format!(".c{i}{{display:block}}"));
            body.push_str("<pre>a\n</pre>");
        }
        let html = page(&css, &body);
        let settled = plain(&html);
        let stop = AtomicBool::new(false);
        let gate = Gate::new(&stop, Duration::from_secs(600));
        let mut tree = Tree::parse(&settled, LIMITS, &gate).unwrap_or_else(|e| panic!("{e}"));
        let mut work = Work {
            steps: 0,
            gate: &gate,
        };
        assert!(matches!(
            rewrite_within(&mut tree, true, &mut work),
            Err(Stop::Budget)
        ));
        assert!(work.steps > MAX_STEPS);
        assert_eq!(safe(&html), settled);
        // And a cancel is seen in the middle of it.
        let stopped = AtomicBool::new(false);
        let cancel = Gate::new(&stopped, Duration::from_secs(600));
        let mut tree = Tree::parse(&settled, LIMITS, &gate).unwrap_or_else(|e| panic!("{e}"));
        let mut work = Work {
            steps: 0,
            gate: &cancel,
        };
        assert!(matches!(facts(&tree, &mut work), Ok(Some(_))));
        stopped.store(true, std::sync::atomic::Ordering::Release);
        assert!(matches!(
            rewrite_within(&mut tree, true, &mut work),
            Err(Stop::Gate(Error::Cancelled))
        ));
    }

    const R: &str = "<rect width=\"100\" height=\"50\" fill=\"#2f6fb0\"/>";
    const PRE: &str = "pre{background:#eef2f6;margin:0 0 10px;font:inherit}";

    /// The reviewer's blocker 1: Chrome attaches a shadow root where the
    /// printer's parser sees an inert template, and its `:host` and
    /// `::slotted()` rules outrank the lowest layer.
    #[test]
    fn a_page_with_a_declarative_shadow_root_is_left_alone() {
        let b1 = page(
            "svg{display:block}",
            &format!(
                "<div><template shadowrootmode=\"open\"><style>::slotted(svg){{width:300px}}</style>\
                 <slot></slot></template><svg width=\"100\" height=\"50\" viewBox=\"0 0 100 50\" \
                 preserveAspectRatio=\"none\">{R}</svg></div>"
            ),
        );
        let f1_host = page(
            "section{padding:4px;border-bottom:4px solid #d33}",
            "<section><template shadowrootmode=\"open\"><style>:host{border-top:4px solid}</style>\
             <slot></slot></template>light</section>",
        );
        let f1_slotted = page(
            ".row{padding:4px 0;border-bottom:4px solid #d33}",
            "<section><template shadowrootmode=\"open\"><style>::slotted(div){border-top:4px solid}\
             </style><slot></slot></template><div class=\"row\">light</div></section>",
        );
        let b2 = page(
            PRE,
            "<pre><span><template shadowrootmode=\"open\"><slot></slot><b>TAIL</b></template>\
             one\n</span></pre>",
        );
        for html in [b1, f1_host, f1_slotted, b2] {
            assert_eq!(safe(&html), plain(&html), "{html}");
        }
        // Any value, any case, and the attribute's older name; anywhere.
        for template in [
            "<template SHADOWROOTMODE=\"Closed\"></template>",
            "<template shadowrootmode></template>",
            "<template shadowroot=\"open\"></template>",
            "<svg><foreignObject><div><template shadowrootmode=\"open\"></template></div></foreignObject></svg>",
        ] {
            let html = page(
                ".row{border-bottom:1px solid #ccc}svg{display:block}",
                &format!("<div>{template}</div>{SVG}{ROWS}<pre>a\n</pre>"),
            );
            assert_eq!(safe(&html), plain(&html), "{template}");
        }
        // A template with no shadow root is inert in Chrome too.
        let inert = page(
            "svg{display:block}",
            &format!("<template x-if=\"a\"><p>x</p></template>{SVG}"),
        );
        assert_eq!(rules_of(&inert).len(), 2);
    }

    /// Assumption 1, the rest of it: markup Chrome's parser reads differently
    /// from the printer's, and elements the pass doesn't know.
    #[test]
    fn a_tree_chrome_may_build_differently_is_left_alone() {
        for extra in [
            "<select><option>a</option></select>",
            "<datalist><option>a</option></datalist>",
            "<template><noscript><p>x</p></noscript></template>",
            "<math><annotation-xml encoding=\"text/html\"><p>x</p></annotation-xml></math>",
            "<ruby>a<rb>b</rb><rtc>c</rtc></ruby>",
            "<slot>x</slot>",
            "<marquee>x</marquee>",
            "<unknownelement>x</unknownelement>",
            "<xmp>x</xmp>",
        ] {
            let html = page(
                ".row{border-bottom:1px solid #ccc}svg{display:block}",
                &format!("{SVG}{ROWS}<pre>a\n</pre>{extra}"),
            );
            assert_eq!(safe(&html), plain(&html), "{extra}");
        }
        for extra in [
            "<my-element some-attr>x</my-element>",
            "<math><mi>x</mi><annotation>y</annotation></math>",
            "<details><summary>s</summary>d</details><dialog>x</dialog>",
            "<table><tr><td>a</td></tr></table><form><input><button>b</button></form>",
        ] {
            let html = page("svg{display:block}", &format!("{SVG}{extra}"));
            assert_eq!(rules_of(&html).len(), 2, "{extra}");
        }
    }

    /// The reviewer's blocker 2: Chrome's legacy names for the logical border
    /// longhands, which set a style without a colour like any other.
    #[test]
    fn a_border_property_the_pass_doesnt_know_stops_the_copies() {
        let row = ".row{padding:4px 0;border-bottom:4px solid #d33}";
        let cases = [
            (
                ".row{padding:4px 0;border-bottom:4px solid #d33;-webkit-border-before-style:solid;\
                 -webkit-border-before-width:4px}"
                    .to_owned(),
                "<div class=\"row\">a</div>",
            ),
            (
                format!(
                    "{row}@keyframes k{{from,to{{-webkit-border-before-width:4px;\
                     -webkit-border-before-style:solid}}}}.row{{animation:k 1s infinite}}"
                ),
                "<div class=\"row\">a</div>",
            ),
            (
                "div{writing-mode:vertical-rl;height:60px;border-bottom:4px solid #d33;\
                 -webkit-border-start-style:solid;-webkit-border-start-width:4px}"
                    .to_owned(),
                "<div>a</div>",
            ),
            (
                format!("{row}.row b{{border-top:4px solid;-webkit-border-before-color:inherit}}"),
                "<div class=\"row\"><b>a</b></div>",
            ),
            // And whatever else has `border` in its name and isn't on the list.
            (format!("{row}p{{-webkit-border-after:1px solid}}"), ROWS),
            (format!("{row}p{{border-start-start-radius:2px}}"), ROWS),
            (format!("{row}p{{border-shape:circle()}}"), ROWS),
            (format!("{row}p{{BORDER-BLOCK-WIDTH:1px}}"), ROWS),
            // `revert` hands a table row its user-agent `border-color: inherit`.
            (
                "table{border-collapse:collapse;border-bottom:4px solid #d33}\
                 tr{border-top:4px solid;border-top-color:revert}"
                    .to_owned(),
                "<table><tr><td>a</td></tr></table>",
            ),
        ];
        for (css, body) in cases {
            let html = page(&css, body);
            assert_eq!(safe(&html), plain(&html), "{css}");
        }
        // Not a property: a custom property's name can be anything.
        let html = page(&format!("{row}:root{{--border-style:solid}}"), ROWS);
        assert_eq!(rules_of(&html), [".row{border-top-color:#d33}"]);
    }

    #[test]
    fn a_colour_is_copied_from_its_own_tokens() {
        // The reviewer's N1: what precedes the token isn't part of it.
        let html = page(
            ".row{border-bottom:4px solid/* } p{color:red} */#d33}",
            ROWS,
        );
        assert_eq!(rules_of(&html), [".row{border-top-color:#d33}"]);
        let html = page(".row{border-bottom:4px solid rgb( 1 , 2,3 )}", ROWS);
        assert_eq!(rules_of(&html), [".row{border-top-color:rgb(1,2,3)}"]);
        for colour in [
            "#d3",
            "#ggg",
            "#12345",
            "rgb(/* x */1,2,3)",
            "rgb(1,2,calc(3))",
            "rgb(1 2 3 / var(--a))",
            "rgb(1e2,2,3)",
        ] {
            let html = page(&format!(".row{{border-bottom:4px solid {colour}}}"), ROWS);
            assert_eq!(added(&html), None, "{colour}");
        }
        // N4: a declaration that may not apply when its rule does.
        let html = page(".row{@media screen{border-bottom:4px solid #d33}}", ROWS);
        assert_eq!(added(&html), None);
        // A selector with a comment in it isn't copied.
        let html = page(".row/* x */{border-bottom:4px solid #d33}", ROWS);
        assert_eq!(added(&html), None);
    }

    #[test]
    fn a_border_is_copied_only_onto_elements_with_plain_attributes() {
        for body in [
            "<div class=\"row\" popover>a</div>",
            "<div class=\"row\" contenteditable>a</div>",
            "<div class=\"row\" futureattribute=\"x\">a</div>",
            "<div class=\"row\">a</div><table frame=\"box\"><tr><td>b</td></tr></table>",
            "<div class=\"row\">a</div><table><tr><td bordercolor=\"red\">b</td></tr></table>",
        ] {
            let html = page(ROW, body);
            assert_eq!(added(&html), None, "{body}");
        }
        let html = page(
            ROW,
            "<div class=\"row\" id=\"a\" data-x=\"1\" aria-label=\"l\" x-data=\"{}\" @click=\"f\" \
             :class=\"c\" title=\"t\" lang=\"ja\" dir=\"ltr\">a</div>\
             <table width=\"100%\" cellpadding=\"2\"><tr><td colspan=\"2\" align=\"right\">b</td></tr></table>",
        );
        assert_eq!(rules_of(&html), [".row{border-top-color:#d9e2ec}"]);
    }

    /// The reviewer's blocker 3: with bidi in play Chrome lays an inline's end
    /// edge out after the forced break, and the newline is what makes that line.
    #[test]
    fn a_newline_stays_where_bidi_may_be_involved() {
        let two = "<pre><code>one\ntwo\n</code></pre><hr>";
        let cases = [
            (
                "code{padding:2px 6px}".to_owned(),
                "<pre><code dir=\"ltr\">one\ntwo\n</code></pre><hr>".to_owned(),
                "",
            ),
            (
                format!("{PRE}code{{unicode-bidi:isolate;border:1px solid #999}}"),
                two.to_owned(),
                "",
            ),
            (
                format!(
                    "{PRE}code{{padding:2px 6px;background:#fd0}}code{{direction:ltr;unicode-bidi:embed}}"
                ),
                two.to_owned(),
                " dir=\"rtl\"",
            ),
            (
                format!("{PRE}span{{padding:0 10px;background:#fd0}}"),
                "<pre>x <span dir=\"rtl\">one!\n</span></pre><hr>".to_owned(),
                "",
            ),
            (
                format!("{PRE}span{{padding:0 10px;background:#fd0}}"),
                "<pre dir=\"rtl\">x <span>one\n</span></pre><hr>".to_owned(),
                "",
            ),
            (
                format!("{PRE}span{{padding:0 10px;background:#fd0}}"),
                "<pre>x <span dir=\"auto\">one!\n</span></pre><hr>".to_owned(),
                "",
            ),
        ];
        for (css, body, html_attributes) in cases {
            let html = page(&css, &body).replace("<html>", &format!("<html{html_attributes}>"));
            assert_eq!(safe(&html), plain(&html), "{css} {body}");
        }
        let span = "<pre>x <span>one\n</span></pre>";
        for (css, body) in [
            // A direction from anywhere: it is inherited.
            ("body{direction:rtl}", span.to_owned()),
            ("p{direction:ltr}", span.to_owned()),
            ("span{unicode-bidi:plaintext}", span.to_owned()),
            ("p{unicode-bidi:isolate-override}", span.to_owned()),
            ("html{writing-mode:vertical-rl}", span.to_owned()),
            ("", format!("<div dir=\"rtl\">{span}</div>")),
            ("", format!("<div dir=\"auto\"><p>{span}</p></div>")),
            ("", format!("<div dir=\"\">{span}</div>")),
            ("", "<pre dir=\"ltr\">x <span>one\n</span></pre>".to_owned()),
            (
                "",
                "<pre>x <a dir=\"ltr\" href=\"#\">one\n</a></pre>".to_owned(),
            ),
            ("", "<pre>x <span lang=\"he\">one\n</span></pre>".to_owned()),
            ("", "<pre wrap>one\n</pre>".to_owned()),
            ("", format!("{span}<p><bdi>x</bdi></p>")),
            ("", format!("<bdo dir=\"ltr\">{span}</bdo>")),
            // Right-to-left text, or a control, anywhere in the `<pre>`.
            (
                "",
                "<pre>\u{5e9}\u{5dc}\u{5d5}\u{5dd} <span>one\n</span></pre>".to_owned(),
            ),
            ("", "<pre>x <span>\u{627}\u{644}!\n</span></pre>".to_owned()),
            ("", "<pre>a\u{202e}b\nc\n</pre>".to_owned()),
            ("", "<pre>a\u{2067}b\n</pre>".to_owned()),
        ] {
            let html = page(css, &body);
            assert_eq!(safe(&html), plain(&html), "{css} {body}");
        }
        // Left to right, said or not: the newline goes.
        for body in [
            format!("<div dir=\"ltr\">{span}</div>"),
            format!("<div dir=\"LTR\"><p>{span}</p></div>"),
            "<p dir=\"rtl\">x</p><pre>x <span class=\"a\" title=\"t\">one\n</span></pre>"
                .to_owned(),
        ] {
            let html = page(PRE, &body);
            assert_eq!(cut(&html).as_deref(), Some("\n"), "{body}");
        }
    }

    /// The reviewer's blocker 4, and assumption 18: the newline goes only
    /// under CSS it was measured under.
    #[test]
    fn a_newline_stays_under_css_it_wasnt_measured_under() {
        let ruby = page(
            &format!("{PRE}span{{display:ruby-text}}b{{display:ruby}}"),
            "<pre><b>x <span>one\n</span></b></pre><hr>",
        );
        assert_eq!(safe(&ruby), plain(&ruby));
        let nested = "<pre class=\"p\"><b>x <span class=\"s\">one\n</span></b></pre>";
        for css in [
            // A `display` on the path that isn't on the list.
            "span{display:ruby}",
            ".s{display:inline-flex}",
            "b{display:table-cell}",
            "pre *{display:list-item}",
            "b{display:var(--d)}",
            "span{all:revert}",
            // A property that isn't.
            "p{-webkit-line-clamp:2}",
            "body{text-combine-upright:all}",
            "p{white-space-collapse:preserve}",
            "p{text-wrap-mode:nowrap}",
            "p{hanging-punctuation:last}",
            "p{some-future-property:1}",
            "@keyframes k{to{text-orientation:upright}}",
            "p{white-space:pre wrap}",
            "p{white-space:var(--w)}",
        ] {
            let html = page(css, nested);
            assert_eq!(safe(&html), plain(&html), "{css}");
        }
        let styled = page("", "<pre><b style=\"display:ruby\">one\n</b></pre>");
        assert_eq!(safe(&styled), plain(&styled));
        let anywhere = page("", "<p style=\"direction:rtl\">x</p><pre>one\n</pre>");
        assert_eq!(safe(&anywhere), plain(&anywhere));
        for css in [
            "span{display:inline-block}b{display:block}",
            ".s{display:contents}.p{display:flow-root}",
            "pre{white-space:pre-wrap;text-wrap:balance;text-align:justify;tab-size:2}",
            "body{font:14px/22px serif;text-spacing-trim:space-all;--anything:direction}",
            "span{padding:0 10px;border:1px solid;margin:0 4px;box-shadow:0 0 2px;outline:1px solid}",
        ] {
            let html = page(css, nested);
            assert_eq!(cut(&html).as_deref(), Some("\n"), "{css}");
        }
        // The size of an `<svg>` doesn't go by what a property means.
        let html = page(
            "svg{display:block}p{some-future-property:1;direction:rtl}",
            SVG,
        );
        assert_eq!(rules_of(&html).len(), 2);
    }

    /// Assumption 4: borders and newlines were measured in standards mode.
    #[test]
    fn a_page_in_quirks_mode_gets_only_its_sizes() {
        let html = page(
            &format!("svg{{display:block}}{ROW}"),
            &format!("{SVG}{ROWS}<pre>a\n</pre>"),
        );
        let quirks = html.replace("<!doctype html>", "");
        let standards = safe(&html);
        assert!(standards.contains(".row{border-top-color:#d9e2ec}}</style>"));
        assert!(standards.contains("<pre>a</pre>"));
        assert_eq!(
            rules_of(&quirks),
            [
                size("width", "640", "640px"),
                size("height", "208", "208px")
            ]
        );
        assert!(safe(&quirks).contains("<pre>a\n</pre>"));
        let bare = "<pre>a\n</pre>";
        assert_eq!(safe(bare), plain(bare));
    }

    #[test]
    fn sizes_are_written_only_for_svgs_the_rule_can_match() {
        // The reviewer's N3.
        for body in [
            format!("<template>{SVG}</template>"),
            format!("<svg><foreignObject><div>{SVG}</div></foreignObject></svg>"),
        ] {
            let html = page("svg{display:block}", &body);
            assert_eq!(added(&html), None, "{body}");
        }
    }

    #[test]
    fn only_a_charset_declares_the_encoding() {
        // The reviewer's N2.
        for meta in [
            "<meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge\">",
            "<meta http-equiv=\"refresh\" content=\"5\">",
            "<meta http-equiv=\"content-type\" content=\"text/html\">",
            "<meta name=\"charset\" content=\"utf-8\">",
        ] {
            let html = format!(
                "<!doctype html><html><head>{meta}<style>p{{}}</style></head><body>売上{SVG}</body></html>"
            );
            assert_eq!(safe(&html), plain(&html), "{meta}");
        }
    }
}
