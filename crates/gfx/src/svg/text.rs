//! Island text to vector outlines (P1-GFX-05).
//!
//! fulgur 0.40 prints inline SVG through usvg with a font database of system
//! fonts only, so a `<text>` would print in whatever fonts the machine has.
//! This pass replaces every SVG `<text>` with `<path>` outlines of the glyphs
//! the registered fonts give, shaped by the same Parley code as
//! `TextMeasurer::measure`, so any engine prints exactly what was measured.
//!
//! The layout follows Chrome, checked against `tests/data/chrome_svg_text.json`
//! (`tests/chrome/svg_text.mjs`): white space collapses across the whole
//! `<text>` (leading and trailing spaces go), `x`/`y`/`dx`/`dy` lists apply
//! per character with descendants overriding ancestors, every absolute `x` or
//! `y` starts an anchored chunk, `letter-spacing` follows every character
//! (the last too), and `dominant-baseline`/`alignment-baseline` shift the
//! baseline by the primary font's rounded metrics, to 1/64 px as Chrome does.
//!
//! Not supported (the text keeps its position on a straight baseline):
//! `rotate`, `textLength`, `<textPath>` (its content is dropped), vertical
//! writing modes, `word-spacing`, `baseline-shift`, text decoration, synthetic
//! bold, and fonts inherited from the page outside the SVG (the defaults are
//! 16px and `serif`). A `<text>` inside `<symbol>` or `<defs>` takes its
//! inherited font from where it is defined, not from the `<use>` that shows
//! it. (`<style>` is removed by the sanitiser, so only presentation
//! attributes and `style` attributes set fonts.)
//!
//! Bounded against hostile islands, which run on the host after the
//! island's script limits: at most [`MAX_OUTLINE_CHARS`] shaped characters
//! (every character that is shaped counts, spaces that survive collapsing
//! too) and [`MAX_OUTLINE_TEXTS`] `<text>` elements per SVG, at most
//! [`MAX_OUTLINE_BYTES`] of path data (checked as it is written, so memory
//! peaks at the budget plus one path command), each element's style parsed
//! once (styles over 64 KiB are not read; the sanitiser drops them), and
//! only glyphs within [`MAX_TEXT_COORD`] px of the origin drawn (beyond that
//! no page shows them, and path numbers stay short). Going over
//! a limit is [`GfxError::InvalidSvg`].

use std::fmt::Write as _;
use std::ops::Range;

use roxmltree::{Document, Node, ParsingOptions};
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{GlyphId, MetadataProvider};

use super::{MAX_SVG_NODES, Ns, Visit, css, escape_attribute, invalid, walk};
use crate::GfxError;
use crate::css_font::{
    FamilyEntry, FontSpec, Slant, parse_family_list, serialize_families, svg_font_size, svg_length,
};
use crate::measure::{Shaped, TextMeasurer};

/// Decimal places in outline path data. A hundredth of a CSS px is far below
/// what a printer or rasteriser resolves, and fixed precision keeps the
/// output byte-identical.
const PATH_DECIMALS: i32 = 2;

/// Most characters one SVG may have outlined. A glyph outline is up to a few
/// KB of path data (dense CJK glyphs), so this bounds the output at tens of
/// MB and the shaping time at well under a second; dense charts have a few
/// thousand characters of labels. More is a hostile or broken island.
pub const MAX_OUTLINE_CHARS: usize = 20_000;

/// Most bytes of outline path data one SVG may produce.
const MAX_OUTLINE_BYTES: usize = 32 * 1024 * 1024;

/// Most `<text>` elements (empty ones too) one SVG may have outlined. Dense
/// charts have a few hundred labels.
const MAX_OUTLINE_TEXTS: usize = 20_000;

/// Text placed farther than this from its user-space origin (px) is not
/// drawn: no page shows it, and it keeps every path number under 12 bytes.
const MAX_TEXT_COORD: f64 = 1e7;

/// Lengths (`x`, `dy`, `letter-spacing`, ...) are clamped to this, far
/// outside [`MAX_TEXT_COORD`], so arithmetic on them stays finite.
const MAX_TEXT_LENGTH: f64 = 1e9;

/// The properties (attribute or `style`) that outlining reads.
const TEXT_PROPERTIES: &[&str] = &[
    "font",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-anchor",
    "dominant-baseline",
    "alignment-baseline",
    "letter-spacing",
    "xml:space",
    "white-space",
];

/// Attributes that position text; they mean nothing on the `<g>` that
/// replaces a `<text>` or `<tspan>`.
const POSITIONING: &[&str] = &["x", "y", "dx", "dy", "rotate", "textlength", "lengthadjust"];

/// Attributes a `<tspan>` ignores but a `<g>` would apply.
const TSPAN_IGNORED: &[&str] = &["transform", "clip-path", "mask", "filter", "opacity"];

/// Replaces every SVG `<text>` in `svg` (already sanitized, well-formed) by
/// outlines. Text inside HTML (`<foreignObject>`) is left alone: fulgur prints
/// HTML text with the registered fonts.
pub(crate) fn outline_text(svg: &str, measurer: &TextMeasurer) -> Result<String, GfxError> {
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
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut inside_text_until = 0;
    let mut budget = Budget::default();
    // Computed styles of the open ancestors (end of element, style), top
    // down: each element's attributes are parsed once.
    let mut styles: Vec<(usize, Style)> = Vec::new();
    walk(root, svg, |visit| {
        if let Visit::Element { node, ns, .. } = visit {
            let r = node.range();
            if r.start < inside_text_until || ns != Ns::Svg {
                return Ok(());
            }
            while styles.last().is_some_and(|(end, _)| *end <= r.start) {
                styles.pop();
            }
            let parent = styles
                .last()
                .map_or_else(Style::default, |(_, s)| s.inherited());
            // An HTML parser lower-cases tag names, so <TEXT> is SVG text too.
            if node.tag_name().name().eq_ignore_ascii_case("text") {
                budget.texts += 1;
                if budget.texts > MAX_OUTLINE_TEXTS {
                    return Err(invalid(format!(
                        "more than {MAX_OUTLINE_TEXTS} text elements to outline"
                    )));
                }
                edits.push((
                    r.clone(),
                    convert_text(node, &parent, svg, measurer, &mut budget)?,
                ));
                inside_text_until = r.end;
            } else {
                let mut own = parent.clone();
                own.apply(node, &parent);
                styles.push((r.end, own));
            }
        }
        Ok(())
    })?;
    let whole = root.range();
    let mut out = String::with_capacity(whole.len() + edits.len() * 512);
    let mut pos = whole.start;
    for (r, new) in &edits {
        out.push_str(&svg[pos..r.start]);
        out.push_str(new);
        pos = r.end;
    }
    out.push_str(&svg[pos..whole.end]);
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Anchor {
    Start,
    Middle,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Baseline {
    Auto,
    Central,
    Middle,
    Hanging,
    TextBeforeEdge,
    TextAfterEdge,
    Mathematical,
}

impl Baseline {
    /// A `dominant-baseline` or `alignment-baseline` keyword. Chrome ignores
    /// `text-top` and `text-bottom` there, so they are `None` too.
    fn parse(value: &str) -> Option<Baseline> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "baseline" | "alphabetic" => Baseline::Auto,
            "central" => Baseline::Central,
            "middle" => Baseline::Middle,
            "hanging" => Baseline::Hanging,
            "text-before-edge" | "before-edge" => Baseline::TextBeforeEdge,
            "text-after-edge" | "after-edge" | "ideographic" => Baseline::TextAfterEdge,
            "mathematical" => Baseline::Mathematical,
            _ => return None,
        })
    }

    /// How far down the glyphs' alphabetic baseline moves, from the primary
    /// font's rounded ascent and descent and its x-height, rounded to 1/64 px
    /// as Chrome's layout units are.
    fn shift(self, shaped: &Shaped) -> f64 {
        let (a, d) = (shaped.ascent, shaped.descent);
        let v = match self {
            Baseline::Auto => 0.0,
            Baseline::Central => (a - d) / 2.0,
            Baseline::Middle => shaped.x_height / 2.0,
            Baseline::Hanging => a * 0.8,
            Baseline::TextBeforeEdge => a,
            Baseline::TextAfterEdge => -d,
            Baseline::Mathematical => a / 2.0,
        };
        (v * 64.0).round() / 64.0
    }
}

/// The text properties that matter for layout, as computed for one element.
#[derive(Clone, Debug)]
struct Style {
    families: Vec<FamilyEntry>,
    size: f64,
    weight: f32,
    slant: Slant,
    anchor: Anchor,
    /// `dominant-baseline` (inherited).
    dominant: Baseline,
    /// `alignment-baseline` (not inherited).
    alignment: Option<Baseline>,
    letter_spacing: f64,
    /// `xml:space="preserve"` or a preserving `white-space`.
    preserve: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            families: vec![FamilyEntry::Generic(crate::GenericFamily::Serif)],
            size: 16.0,
            weight: 400.0,
            slant: Slant::Normal,
            anchor: Anchor::Start,
            dominant: Baseline::Auto,
            alignment: None,
            letter_spacing: 0.0,
            preserve: false,
        }
    }
}

impl Style {
    /// The style a child element starts from.
    fn inherited(&self) -> Style {
        Style {
            alignment: None,
            ..self.clone()
        }
    }

    /// The CSS `font` shorthand for this style, for the measurer.
    fn css_font(&self) -> String {
        let slant = match self.slant {
            Slant::Normal => String::new(),
            Slant::Italic => "italic ".into(),
            Slant::Oblique(None) => "oblique ".into(),
            Slant::Oblique(Some(deg)) => format!("oblique {deg}deg "),
        };
        format!(
            "{slant}{} {}px {}",
            self.weight,
            self.size,
            serialize_families(&self.families)
        )
    }

    /// Applies an element's presentation attributes, then its `style`
    /// declarations (which win), to the style it inherited.
    fn apply(&mut self, node: Node<'_, '_>, parent: &Style) {
        let mut decls: Vec<(String, String)> = Vec::new();
        for a in node.attributes() {
            let name = if a.namespace() == Some("http://www.w3.org/XML/1998/namespace")
                && a.name() == "space"
            {
                "xml:space".to_string()
            } else if a.namespace().is_none() {
                a.name().to_ascii_lowercase()
            } else {
                continue;
            };
            if TEXT_PROPERTIES.contains(&name.as_str()) {
                decls.push((name, a.value().to_string()));
            }
        }
        if let Some(style) = node.attribute("style") {
            decls.extend(css::declaration_pairs(style, |n| {
                TEXT_PROPERTIES.contains(&n)
            }));
        }
        // Font size first: `em` in the other properties refers to it.
        for (name, value) in &decls {
            if name == "font-size" || name == "font" {
                self.property(name, value, parent);
            }
        }
        for (name, value) in &decls {
            if name != "font-size" && name != "font" {
                self.property(name, value, parent);
            }
        }
    }

    fn property(&mut self, name: &str, value: &str, parent: &Style) {
        let value = value.trim();
        let lower = value.to_ascii_lowercase();
        if lower == "inherit" {
            match name {
                "font-family" => self.families = parent.families.clone(),
                "font-size" => self.size = parent.size,
                "font-weight" => self.weight = parent.weight,
                "font-style" => self.slant = parent.slant,
                "text-anchor" => self.anchor = parent.anchor,
                "dominant-baseline" => self.dominant = parent.dominant,
                "letter-spacing" => self.letter_spacing = parent.letter_spacing,
                _ => {}
            }
            return;
        }
        match name {
            "font" => {
                if let Some(spec) = FontSpec::parse(value) {
                    self.families = spec.families;
                    self.size = spec.size_px as f64;
                    self.weight = spec.weight;
                    self.slant = spec.slant;
                }
            }
            "font-family" => {
                if let Some(families) = parse_family_list(value).filter(|f| !f.is_empty()) {
                    self.families = families;
                }
            }
            "font-size" => {
                if let Some(px) = svg_font_size(value, parent.size) {
                    self.size = px;
                }
            }
            "font-weight" => {
                self.weight = match lower.as_str() {
                    "normal" => 400.0,
                    "bold" => 700.0,
                    // CSS Fonts 4, relative weights.
                    "bolder" => match parent.weight {
                        w if w < 350.0 => 400.0,
                        w if w < 550.0 => 700.0,
                        w if w < 900.0 => 900.0,
                        w => w,
                    },
                    "lighter" => match parent.weight {
                        w if w < 100.0 => w,
                        w if w < 550.0 => 100.0,
                        w if w < 750.0 => 400.0,
                        _ => 700.0,
                    },
                    _ => match lower.parse::<f32>() {
                        Ok(w) if (1.0..=1000.0).contains(&w) => w,
                        _ => self.weight,
                    },
                }
            }
            "font-style" => {
                self.slant = match lower.as_str() {
                    "normal" => Slant::Normal,
                    "italic" => Slant::Italic,
                    s if s.starts_with("oblique") => Slant::Oblique(None),
                    _ => self.slant,
                }
            }
            "text-anchor" => {
                self.anchor = match lower.as_str() {
                    "start" => Anchor::Start,
                    "middle" => Anchor::Middle,
                    "end" => Anchor::End,
                    _ => self.anchor,
                }
            }
            "dominant-baseline" => {
                if let Some(b) = Baseline::parse(&lower) {
                    self.dominant = b;
                }
            }
            "alignment-baseline" => self.alignment = Baseline::parse(&lower),
            "letter-spacing" => {
                if lower == "normal" {
                    self.letter_spacing = 0.0;
                } else if let Some(px) = svg_length(&lower, self.size).filter(|v| v.is_finite()) {
                    self.letter_spacing = px.clamp(-MAX_TEXT_LENGTH, MAX_TEXT_LENGTH);
                }
            }
            "xml:space" => self.preserve = lower == "preserve",
            "white-space" => {
                self.preserve = matches!(lower.as_str(), "pre" | "pre-wrap" | "break-spaces");
            }
            _ => {}
        }
    }
}

/// The style a `<text>` inherits from its ancestors (for tests; outlining
/// computes styles top down instead).
#[cfg(test)]
fn inherited_style(text: Node<'_, '_>) -> Style {
    let mut chain: Vec<Node<'_, '_>> = text
        .ancestors()
        .skip(1)
        .filter(|n| n.is_element())
        .collect();
    chain.reverse();
    let mut style = Style::default();
    for node in chain {
        let parent = style.inherited();
        let mut own = parent.clone();
        own.apply(node, &parent);
        style = own;
    }
    style.inherited()
}

/// One element of the text subtree with its character range and lists.
struct Positioned {
    chars: Range<usize>,
    x: Vec<f64>,
    y: Vec<f64>,
    dx: Vec<f64>,
    dy: Vec<f64>,
}

/// A text node's characters and computed style.
struct TextRun {
    chars: Range<usize>,
    style: Style,
}

/// Everything collected from one `<text>` before layout.
#[derive(Default)]
struct Collected {
    chars: Vec<char>,
    /// Per character: may it collapse away (not preserved white space)?
    collapsible: Vec<bool>,
    /// Per character: index into `runs`.
    run_of: Vec<usize>,
    runs: Vec<TextRun>,
    elements: Vec<Positioned>,
    /// The last character is a collapsible space (leading spaces go too).
    after_space: bool,
}

fn is_named(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element() && node.tag_name().name().eq_ignore_ascii_case(name)
}

/// A length list (`x="10 20,30"`); all of it is ignored if any item is
/// invalid. Values are clamped to [`MAX_TEXT_LENGTH`]. Only as many items as
/// the element has characters matter: the rest are not kept.
fn length_list(value: Option<&str>, em: f64, max_items: usize) -> Vec<f64> {
    let Some(value) = value else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for t in value
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
    {
        match svg_length(t, em).filter(|v| v.is_finite()) {
            Some(v) if out.len() < max_items => {
                out.push(v.clamp(-MAX_TEXT_LENGTH, MAX_TEXT_LENGTH))
            }
            Some(_) => {}
            None => return Vec::new(),
        }
    }
    out
}

/// Collects the characters of `node` and its `<tspan>`s. Fails once more
/// than `limit` characters would be shaped.
fn collect(
    node: Node<'_, '_>,
    style: &Style,
    out: &mut Collected,
    limit: usize,
) -> Result<(), GfxError> {
    let slot = out.elements.len();
    let start = out.chars.len();
    let max_items = limit + 1;
    out.elements.push(Positioned {
        chars: start..start,
        x: length_list(node.attribute("x"), style.size, max_items),
        y: length_list(node.attribute("y"), style.size, max_items),
        dx: length_list(node.attribute("dx"), style.size, max_items),
        dy: length_list(node.attribute("dy"), style.size, max_items),
    });
    for child in node.children() {
        if child.is_text() {
            let run = out.runs.len();
            let first = out.chars.len();
            for c in child.text().unwrap_or("").chars() {
                // Every character that will be shaped counts, white space
                // too (one more for a trailing space that may still go).
                if out.chars.len() > limit {
                    return Err(too_many_chars());
                }
                let c = if matches!(c, '\n' | '\t' | '\r') {
                    ' '
                } else {
                    c
                };
                if !style.preserve && c == ' ' && out.after_space {
                    continue;
                }
                out.after_space = !style.preserve && c == ' ';
                out.chars.push(c);
                out.collapsible.push(!style.preserve);
                out.run_of.push(run);
            }
            out.runs.push(TextRun {
                chars: first..out.chars.len(),
                style: style.clone(),
            });
        } else if is_named(child, "tspan") || is_named(child, "a") {
            let parent = style.inherited();
            let mut own = parent.clone();
            own.apply(child, &parent);
            collect(child, &own, out, limit)?;
        }
        // <textPath>, <title>, <desc> and anything else: no rendered characters here.
    }
    out.elements[slot].chars.end = out.chars.len();
    Ok(())
}

fn too_many_chars() -> GfxError {
    invalid(format!(
        "more than {MAX_OUTLINE_CHARS} characters of text to outline"
    ))
}

/// A shaped piece of text placed in the `<text>`'s user space.
struct Segment {
    /// Index of the text run (text node) it comes from.
    run: usize,
    /// Characters of the `<text>` this segment holds (read by the Chrome
    /// position test).
    #[cfg_attr(not(test), allow(dead_code))]
    chars: Range<usize>,
    x: f64,
    y: f64,
    shaped: Shaped,
    /// Baseline shift (down) from `dominant-baseline`/`alignment-baseline`.
    shift: f64,
}

/// The laid-out `<text>`: its segments and the string it shows.
struct Laid {
    segments: Vec<Segment>,
    label: String,
    runs: Vec<TextRun>,
}

/// Lays out a `<text>` whose ancestors give it `parent`, shaping at most
/// `limit` characters.
fn layout(
    text: Node<'_, '_>,
    parent: &Style,
    measurer: &TextMeasurer,
    limit: usize,
) -> Result<Laid, GfxError> {
    let mut style = parent.clone();
    style.apply(text, parent);
    // Leading spaces are dropped as if a space came before.
    let mut c = Collected {
        after_space: true,
        ..Collected::default()
    };
    collect(text, &style, &mut c, limit)?;
    // A trailing collapsible space goes.
    if c.chars.last() == Some(&' ') && c.collapsible.last() == Some(&true) {
        let n = c.chars.len() - 1;
        c.chars.truncate(n);
        c.run_of.truncate(n);
        for r in &mut c.runs {
            r.chars.end = r.chars.end.min(n);
            r.chars.start = r.chars.start.min(n);
        }
        for e in &mut c.elements {
            e.chars.end = e.chars.end.min(n);
            e.chars.start = e.chars.start.min(n);
        }
    }
    let n = c.chars.len();
    if n > limit {
        return Err(too_many_chars());
    }
    // Positions per character; elements come in document order, so a
    // descendant's list overrides its ancestors'.
    let mut abs_x = vec![None; n];
    let mut abs_y = vec![None; n];
    let mut rel_x = vec![None; n];
    let mut rel_y = vec![None; n];
    for e in &c.elements {
        for (list, target) in [
            (&e.x, &mut abs_x),
            (&e.y, &mut abs_y),
            (&e.dx, &mut rel_x),
            (&e.dy, &mut rel_y),
        ] {
            for (k, v) in list.iter().enumerate() {
                match target.get_mut(e.chars.start + k) {
                    Some(slot) if e.chars.start + k < e.chars.end => *slot = Some(*v),
                    _ => break,
                }
            }
        }
    }

    let mut segments: Vec<Segment> = Vec::new();
    // (first segment, anchor) of each chunk.
    let mut chunks: Vec<(usize, Anchor)> = Vec::new();
    let (mut x, mut y) = (0.0f64, 0.0f64);
    let mut i = 0;
    while i < n {
        let run = c.run_of[i];
        let mut end = i + 1;
        while end < n
            && c.run_of[end] == run
            && abs_x[end].is_none()
            && abs_y[end].is_none()
            && rel_x[end].is_none()
            && rel_y[end].is_none()
        {
            end += 1;
        }
        if i == 0 || abs_x[i].is_some() || abs_y[i].is_some() {
            chunks.push((segments.len(), c.runs[run].style.anchor));
        }
        x = abs_x[i].unwrap_or(x) + rel_x[i].unwrap_or(0.0);
        y = abs_y[i].unwrap_or(y) + rel_y[i].unwrap_or(0.0);
        let st = &c.runs[run].style;
        let s: String = c.chars[i..end].iter().collect();
        // At most MAX_OUTLINE_CHARS characters: never over MAX_TEXT_BYTES.
        let shaped = measurer
            .shape(&s, &st.css_font(), st.letter_spacing as f32)
            .map_err(|e| match e {
                GfxError::TextTooLong { .. } => too_many_chars(),
                e => e,
            })?;
        let shift = st.alignment.unwrap_or(st.dominant).shift(&shaped);
        let advance = shaped.advance;
        segments.push(Segment {
            run,
            chars: i..end,
            x,
            y,
            shaped,
            shift,
        });
        x += advance;
        i = end;
    }
    // Anchor each chunk by its extent.
    for (k, &(first, anchor)) in chunks.iter().enumerate() {
        let last = chunks.get(k + 1).map_or(segments.len(), |c| c.0);
        let Some(start) = segments.get(first).map(|s| s.x) else {
            continue;
        };
        let end = segments[first..last]
            .iter()
            .map(|s| s.x + s.shaped.advance)
            .fold(start, f64::max);
        let dx = match anchor {
            Anchor::Start => 0.0,
            Anchor::Middle => -(end - start) / 2.0,
            Anchor::End => -(end - start),
        };
        for s in &mut segments[first..last] {
            s.x += dx;
        }
    }
    Ok(Laid {
        segments,
        label: c.chars.iter().collect(),
        runs: c.runs,
    })
}

/// True if a glyph origin lies within [`MAX_TEXT_COORD`] of the origin.
fn near(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_TEXT_COORD
}

/// Outline path data for a run's glyphs, placed at `(x, y)` on the baseline.
/// Glyphs placed beyond [`MAX_TEXT_COORD`] are not drawn. Stops writing once
/// `budget` is spent.
fn draw_segment(seg: &Segment, out: &mut String, budget: &mut ByteBudget) {
    let (ox, oy) = (seg.x, seg.y + seg.shift);
    for run in &seg.shaped.runs {
        let Ok(font) = skrifa::FontRef::from_index(run.font.data.as_ref(), run.font.index) else {
            continue;
        };
        let outlines = font.outline_glyphs();
        let coords: Vec<NormalizedCoord> = run
            .coords
            .iter()
            .map(|&c| NormalizedCoord::from_bits(c))
            .collect();
        let location = LocationRef::new(&coords);
        for &(id, gx, gy) in &run.glyphs {
            if budget.over {
                return;
            }
            if !(near(ox + gx) && near(oy + gy)) {
                continue;
            }
            let Some(glyph) = outlines.get(GlyphId::new(id)) else {
                continue;
            };
            let mut pen = PathPen {
                out,
                x: ox + gx,
                y: oy + gy,
                skew: run.skew,
                budget,
            };
            // Unhinted outlines at the font size: exact, and the same at any scale.
            let _ = glyph.draw(
                DrawSettings::unhinted(Size::new(run.size), location),
                &mut pen,
            );
        }
    }
}

/// Bytes of path data one SVG may still write.
struct ByteBudget {
    left: usize,
    over: bool,
}

/// Writes SVG path commands, y flipped (fonts are y up) and offset, and
/// stops once the budget is spent: memory peaks at the budget plus one
/// command (under 100 bytes, as every number is under 12).
struct PathPen<'a> {
    out: &'a mut String,
    x: f64,
    y: f64,
    skew: f64,
    budget: &'a mut ByteBudget,
}

impl PathPen<'_> {
    fn point(&mut self, x: f32, y: f32) {
        let (x, y) = (x as f64, y as f64);
        push_number(self.out, self.x + x + self.skew * y);
        self.out.push(' ');
        push_number(self.out, self.y - y);
    }

    /// Writes one command with `points`, if the budget allows.
    fn command(&mut self, letter: char, points: &[(f32, f32)]) {
        if self.budget.over {
            return;
        }
        let before = self.out.len();
        self.out.push(letter);
        for (k, &(x, y)) in points.iter().enumerate() {
            if k > 0 {
                self.out.push(' ');
            }
            self.point(x, y);
        }
        let used = self.out.len() - before;
        if used > self.budget.left {
            self.budget.over = true;
        } else {
            self.budget.left -= used;
        }
    }
}

impl OutlinePen for PathPen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.command('M', &[(x, y)]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.command('L', &[(x, y)]);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.command('Q', &[(cx0, cy0), (x, y)]);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.command('C', &[(cx0, cy0), (cx1, cy1), (x, y)]);
    }
    fn close(&mut self) {
        self.command('Z', &[]);
    }
}

/// A number with at most [`PATH_DECIMALS`] decimals, no trailing zeros and
/// never `-0`.
fn push_number(out: &mut String, v: f64) {
    let scale = 10f64.powi(PATH_DECIMALS);
    let r = (v * scale).round() / scale;
    if r == 0.0 || !r.is_finite() {
        out.push('0');
        return;
    }
    let start = out.len();
    let _ = write!(out, "{:.*}", PATH_DECIMALS as usize, r);
    while out.ends_with('0') {
        out.pop();
    }
    if out.ends_with('.') {
        out.pop();
    }
    debug_assert!(out.len() > start);
}

/// The start tag of `node` renamed to `name` (keeping a namespace prefix),
/// without the `drop` attributes, with `extra` appended; and whether the
/// original was self-closing.
fn start_tag(
    node: Node<'_, '_>,
    input: &str,
    name: &str,
    drop: &[&str],
    extra: &str,
) -> (String, bool) {
    let start = node.range().start;
    let qname_len = input[start + 1..]
        .find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
        .unwrap_or(0);
    let qname = &input[start + 1..start + 1 + qname_len];
    let prefix = qname.rfind(':').map_or("", |k| &qname[..=k]);
    let mut pos = start + 1 + qname_len;
    let attrs_end = node
        .attributes()
        .map(|a| a.range().end)
        .fold(pos, usize::max);
    let mut out = format!("<{prefix}{name}");
    for a in node.attributes() {
        let r = a.range();
        let drop_it =
            a.namespace().is_none() && drop.contains(&a.name().to_ascii_lowercase().as_str());
        if drop_it {
            out.push_str(input[pos..r.start].trim_end());
        } else {
            out.push_str(&input[pos..r.end]);
        }
        pos = r.end;
    }
    let tag_end = input[attrs_end..]
        .find('>')
        .map_or(input.len(), |k| attrs_end + k);
    let self_closing = input[attrs_end..tag_end].trim_end().ends_with('/');
    out.push_str(extra);
    out.push_str(if self_closing { "/>" } else { ">" });
    (out, self_closing)
}

/// What one SVG has used of [`MAX_OUTLINE_CHARS`], [`MAX_OUTLINE_TEXTS`]
/// and [`MAX_OUTLINE_BYTES`].
struct Budget {
    chars: usize,
    texts: usize,
    bytes: ByteBudget,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            chars: 0,
            texts: 0,
            bytes: ByteBudget {
                left: MAX_OUTLINE_BYTES,
                over: false,
            },
        }
    }
}

/// Outline paths keep the nonzero rule glyphs need, whatever the island or
/// the page sets (`!important` in `style` beats any author rule).
const NONZERO_FILL: &str = "fill-rule:nonzero!important";
const NONZERO_CLIP: &str = "clip-rule:nonzero!important";

fn convert_text(
    text: Node<'_, '_>,
    parent: &Style,
    input: &str,
    measurer: &TextMeasurer,
    budget: &mut Budget,
) -> Result<String, GfxError> {
    let limit = MAX_OUTLINE_CHARS - budget.chars.min(MAX_OUTLINE_CHARS);
    let laid = layout(text, parent, measurer, limit)?;
    budget.chars += laid.label.chars().count();
    // Path data per text run.
    let mut paths = vec![String::new(); laid.runs.len()];
    for seg in &laid.segments {
        draw_segment(seg, &mut paths[seg.run], &mut budget.bytes);
    }
    if budget.bytes.over {
        return Err(invalid(format!(
            "outlined text would exceed {} MiB",
            MAX_OUTLINE_BYTES / (1024 * 1024)
        )));
    }
    let in_clip_path = text.ancestors().any(|a| is_named(a, "clipPath"));
    if in_clip_path {
        // A clipPath may hold shapes but no <g>: one path for all the text.
        // Its own rule comes first, so no unclosed block in the text's style
        // can swallow it.
        let d: String = paths.concat();
        let mut drop: Vec<&str> = POSITIONING.to_vec();
        drop.extend(["aria-label", "style", "data-surimono-text"]);
        let style = match text.attribute("style") {
            Some(s) if !s.trim().is_empty() => format!("{NONZERO_CLIP};{s}"),
            _ => NONZERO_CLIP.to_string(),
        };
        let extra = format!(" style=\"{}\" d=\"{d}\"", escape_attribute(&style, '"'));
        let (mut tag, self_closing) = start_tag(text, input, "path", &drop, &extra);
        if !self_closing {
            tag.truncate(tag.len() - 1);
            tag.push_str("/>");
        }
        return Ok(tag);
    }
    // The outlined string is always recorded (the text checks read it); the
    // author's own label and role, if any, stay.
    let mut extra = format!(
        " data-surimono-text=\"{}\"",
        escape_attribute(&laid.label, '"')
    );
    if text.attribute("aria-label").is_none() {
        if text.attribute("role").is_none() {
            extra.push_str(" role=\"img\"");
        }
        extra.push_str(&format!(
            " aria-label=\"{}\"",
            escape_attribute(&laid.label, '"')
        ));
    }
    let mut drop: Vec<&str> = POSITIONING.to_vec();
    drop.push("data-surimono-text");
    let (mut out, self_closing) = start_tag(text, input, "g", &drop, &extra);
    if self_closing {
        return Ok(out);
    }
    let mut run = 0;
    emit_children(text, input, &paths, &mut run, &mut out);
    out.push_str(&format!("</{}>", end_qname(text, input, "g")));
    Ok(out)
}

/// `g` (or `prefix:g`) for an element written with a namespace prefix.
fn end_qname(node: Node<'_, '_>, input: &str, name: &str) -> String {
    let start = node.range().start + 1;
    let qname_len = input[start..]
        .find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>')
        .unwrap_or(0);
    let qname = &input[start..start + qname_len];
    match qname.rfind(':') {
        Some(k) => format!("{}{name}", &qname[..=k]),
        None => name.to_string(),
    }
}

fn emit_children(
    node: Node<'_, '_>,
    input: &str,
    paths: &[String],
    run: &mut usize,
    out: &mut String,
) {
    for child in node.children() {
        if child.is_text() {
            let d = paths.get(*run).map_or("", String::as_str);
            *run += 1;
            if !d.is_empty() {
                out.push_str("<path style=\"");
                out.push_str(NONZERO_FILL);
                out.push_str("\" d=\"");
                out.push_str(d);
                out.push_str("\"/>");
            }
        } else if is_named(child, "tspan") {
            // What a tspan ignores but a <g> would apply goes, from its
            // attributes and from its style.
            let mut drop: Vec<&str> = POSITIONING.to_vec();
            drop.extend_from_slice(TSPAN_IGNORED);
            drop.push("style");
            let extra = match child.attribute("style") {
                Some(s) => {
                    let kept = css::remove_properties(s, TSPAN_IGNORED);
                    if kept.trim().is_empty() {
                        String::new()
                    } else {
                        format!(" style=\"{}\"", escape_attribute(&kept, '"'))
                    }
                }
                None => String::new(),
            };
            let (tag, self_closing) = start_tag(child, input, "g", &drop, &extra);
            out.push_str(&tag);
            if !self_closing {
                emit_children(child, input, paths, run, out);
                out.push_str(&format!("</{}>", end_qname(child, input, "g")));
            }
        } else if is_named(child, "a") {
            let (tag, self_closing) = start_tag(child, input, "a", &[], "");
            out.push_str(&tag);
            if !self_closing {
                emit_children(child, input, paths, run, out);
                out.push_str(&format!("</{}>", end_qname(child, input, "a")));
            }
        } else if is_named(child, "title") || is_named(child, "desc") {
            // Already sanitized; keeps the text's accessible description.
            out.push_str(&input[child.range()]);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::FontSet;

    pub(crate) fn test_measurer() -> TextMeasurer {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/fonts/");
        let mut fonts = FontSet::new();
        for f in [
            "NotoSans-Regular.ttf",
            "NotoSans-Bold.ttf",
            "NotoSansJP-Regular.otf",
            "NotoSansJP-Bold.otf",
        ] {
            fonts
                .add_font_bytes(std::fs::read(format!("{dir}{f}")).unwrap())
                .unwrap();
        }
        TextMeasurer::new(fonts)
    }

    /// Chrome positions SVG text in 1/64 px layout units and rounds each
    /// advance to them, so its positions drift from the exact (Parley, float)
    /// ones by up to 1/128 px per character before. Parley's own advances
    /// equal measureText's to 0.0007 px (tests/measure_text.rs).
    const X_TOLERANCE_PER_CHAR: f64 = 1.0 / 64.0;
    /// Baselines: both sides round the same metrics; Chrome to 1/64 px.
    const BASELINE_TOLERANCE: f64 = 1.0 / 64.0;

    /// Every character of every `<text>` in `tests/data/chrome_svg_text.json`
    /// (Chromium 153, `tests/chrome/svg_text.mjs`) lands where Chrome puts it.
    #[test]
    fn layout_matches_chrome() {
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/data/chrome_svg_text.json")).unwrap();
        let m = test_measurer();
        let (mut worst_x, mut worst_base, mut count) = (0.0f64, 0.0f64, 0);
        let mut failures = Vec::new();
        for case in reference["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\">{}</svg>",
                case["svg"].as_str().unwrap()
            );
            let doc = Document::parse(&svg).unwrap();
            let texts: Vec<_> = doc.descendants().filter(|n| is_named(*n, "text")).collect();
            let expected = case["texts"].as_array().unwrap();
            assert_eq!(texts.len(), expected.len(), "{name}");
            for (node, want) in texts.into_iter().zip(expected) {
                let laid = layout(node, &inherited_style(node), &m, MAX_OUTLINE_CHARS).unwrap();
                let chars = want["chars"].as_array().unwrap();
                assert_eq!(
                    laid.label.chars().count(),
                    chars.len(),
                    "{name}: {:?} vs Chrome's {} characters",
                    laid.label,
                    chars.len()
                );
                for seg in &laid.segments {
                    for (k, i) in seg.chars.clone().enumerate() {
                        let c = &seg.shaped.chars[k];
                        let w = &chars[i];
                        let f = |v: &serde_json::Value, j: usize| {
                            v.as_array().unwrap()[j].as_f64().unwrap()
                        };
                        let (sx, ex) = (f(&w["start"], 0), f(&w["end"], 0));
                        let (ey, eh) = (f(&w["extent"], 1), f(&w["extent"], 3));
                        let start = seg.x + c.x;
                        let end = start + c.advance;
                        let baseline = seg.y + seg.shift;
                        let tol_x = X_TOLERANCE_PER_CHAR * (i + 1) as f64;
                        let dx = (start - sx).abs().max((end - ex).abs());
                        // Chrome's extent is the primary font's box, for
                        // fallback glyphs too.
                        let (a, d) = (seg.shaped.ascent, seg.shaped.descent);
                        let db = (baseline - a - ey).abs();
                        let dh = (a + d - eh).abs();
                        worst_x = worst_x.max(dx);
                        worst_base = worst_base.max(db);
                        count += 1;
                        if dx > tol_x || db > BASELINE_TOLERANCE || dh > 0.0 {
                            failures.push(format!(
                                "{name} {:?}[{i}]: start {start:.4} end {end:.4} baseline {baseline:.4} box {} vs Chrome start {sx} end {ex} extent y {ey} h {eh}",
                                laid.label,
                                a + d
                            ));
                        }
                    }
                }
            }
        }
        println!(
            "{count} characters: max x deviation {worst_x:.4} px, max baseline deviation {worst_base:.4} px"
        );
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Outlines advance exactly as measureText measures: same shaping code,
    /// same bits, for Latin, Japanese and mixed text.
    #[test]
    fn outline_advance_equals_measure() {
        let m = test_measurer();
        for (text, font) in [
            ("Revenue 2026", "12px 'Noto Sans'"),
            ("AVATAR office", "bold 13.5px 'Noto Sans'"),
            ("売上高（百万円）", "14px 'Noto Sans JP'"),
            ("Sales 売上 2026年", "12px 'Noto Sans', 'Noto Sans JP'"),
            ("Italic 日本", "italic 16px 'Noto Sans'"),
        ] {
            let measured = m.measure(text, font).unwrap().width;
            let shaped = m.shape(text, font, 0.0).unwrap().advance;
            assert_eq!(shaped.to_bits(), measured.to_bits(), "{text} in {font}");
            // Through the SVG path: a <text> anchored at its end starts
            // exactly one measured width to the left.
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\"><text x=\"100\" text-anchor=\"end\" style=\"font:{}\">{text}</text></svg>",
                font.replace('\'', "&apos;")
            );
            let doc = Document::parse(&svg).unwrap();
            let node = doc.descendants().find(|n| is_named(*n, "text")).unwrap();
            let laid = layout(node, &inherited_style(node), &m, MAX_OUTLINE_CHARS).unwrap();
            assert_eq!(laid.segments.len(), 1);
            assert_eq!(laid.segments[0].x, 100.0 - measured, "{text}");
        }
    }

    #[test]
    fn output_keeps_structure_paint_and_meaning() {
        let m = test_measurer();
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\"><g fill-rule=\"evenodd\">\
            <text id=\"t\" class=\"c\" x=\"10\" y=\"20\" fill=\"red\" transform=\"rotate(5)\" font-family=\"'Noto Sans'\" font-size=\"12\">Hi &amp; <tspan font-weight=\"bold\" fill=\"blue\" dx=\"3\" opacity=\"0.5\" style=\"opacity:.2;transform:scale(9);fill:green;filter:blur(1px)\">bye</tspan><title>tip</title></text>\
            <clipPath id=\"k\"><text x=\"1\" y=\"9\" font-size=\"8\" style=\"fill:red\">clip</text></clipPath><text/>\
            <text aria-label=\"Revenue: 12\" role=\"graphics-symbol\" data-surimono-text=\"forged\">12</text></g></svg>";
        let out = outline_text(svg, &m).unwrap();
        let doc = Document::parse(&out).unwrap();
        assert!(
            !doc.descendants()
                .any(|n| is_named(n, "text") || is_named(n, "tspan"))
        );
        let g = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("t"))
            .unwrap();
        assert!(g.has_tag_name("g"));
        assert_eq!(g.attribute("class"), Some("c"));
        assert_eq!(g.attribute("fill"), Some("red"));
        assert_eq!(g.attribute("transform"), Some("rotate(5)"));
        assert_eq!(g.attribute("role"), Some("img"));
        assert_eq!(g.attribute("aria-label"), Some("Hi & bye"));
        assert_eq!(g.attribute("data-surimono-text"), Some("Hi & bye"));
        assert_eq!(g.attribute("x"), None);
        let kids: Vec<_> = g.children().filter(|n| n.is_element()).collect();
        assert_eq!(kids.len(), 3, "path, tspan group, title");
        assert!(kids[0].has_tag_name("path"));
        // Glyphs need nonzero, whatever the island or the page says.
        assert_eq!(
            kids[0].attribute("style"),
            Some("fill-rule:nonzero!important")
        );
        assert_eq!(kids[0].attribute("fill-rule"), None);
        assert!(kids[1].has_tag_name("g"));
        assert_eq!(kids[1].attribute("fill"), Some("blue"));
        assert_eq!(kids[1].attribute("dx"), None);
        assert_eq!(
            kids[1].attribute("opacity"),
            None,
            "opacity does not apply to tspan"
        );
        assert_eq!(
            kids[1].attribute("style"),
            Some(";;fill:green;"),
            "nor in its style"
        );
        assert!(kids[2].has_tag_name("title"));
        // In a clipPath, one bare path, with the clip rule first.
        let clip = doc
            .descendants()
            .find(|n| n.attribute("id") == Some("k"))
            .unwrap();
        let p = clip.first_element_child().unwrap();
        assert!(p.has_tag_name("path") && p.attribute("d").unwrap().len() > 20);
        assert_eq!(
            p.attribute("style"),
            Some("clip-rule:nonzero!important;fill:red")
        );
        // An empty <text/> stays an (empty) labelled group.
        assert!(out.contains("<g data-surimono-text=\"\" role=\"img\" aria-label=\"\"/>"));
        // The author's label and role stay; the outlined string is always
        // recorded, and a forged one goes.
        let v = doc
            .descendants()
            .find(|n| n.attribute("aria-label") == Some("Revenue: 12"))
            .unwrap();
        assert_eq!(v.attribute("role"), Some("graphics-symbol"));
        assert_eq!(v.attribute("data-surimono-text"), Some("12"));
    }

    fn wrap(body: &str) -> String {
        format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{body}</svg>")
    }

    fn is_too_much(r: Result<String, GfxError>) -> bool {
        matches!(r, Err(GfxError::InvalidSvg { .. }))
    }

    /// A flood of text is a typed error before it is shaped.
    #[test]
    fn too_much_text_is_an_error_not_a_blowup() {
        let m = test_measurer();
        let svg = wrap(&"<text>鬱</text>".repeat(MAX_OUTLINE_CHARS + 1));
        assert!(is_too_much(outline_text(&svg, &m)));
        // Just under the limit is fine; spaces between words count.
        let svg = wrap(&format!(
            "<text>{}</text>",
            "a ".repeat(MAX_OUTLINE_CHARS / 2)
        ));
        assert!(outline_text(&svg, &m).is_ok());
        let svg = wrap(&format!(
            "<text>{}</text>",
            "a ".repeat(MAX_OUTLINE_CHARS / 2 + 1)
        ));
        assert!(is_too_much(outline_text(&svg, &m)));
    }

    /// Review of #27, finding 3: white space that is shaped counts, so
    /// preserved spaces and spaces that never collapse can't flood.
    #[test]
    fn shaped_white_space_counts() {
        let m = test_measurer();
        let n = MAX_OUTLINE_CHARS + 1;
        for body in [
            format!("<text>{}</text>", "\u{a0}".repeat(n)),
            format!("<text>{}</text>", "\u{3000}".repeat(n)),
            format!("<text xml:space=\"preserve\">{}</text>", " ".repeat(n + 1)),
            format!(
                "<text style=\"white-space:pre\">{}</text>",
                "\t".repeat(n + 1)
            ),
            // 80 MB of them would be refused just as fast: collection stops
            // at the limit.
            format!("<text>{}</text>", "\u{a0}".repeat(4 * n)),
        ] {
            assert!(is_too_much(outline_text(&wrap(&body), &m)));
        }
        // Collapsing spaces cost one each, at most.
        let svg = wrap(&format!("<text>a{}b</text>", " ".repeat(10 * n)));
        assert!(outline_text(&svg, &m).is_ok());
    }

    /// Review of #27, finding 1: huge coordinates made every path number
    /// about 300 digits. Text out of range is not drawn, numbers stay short,
    /// and the byte budget holds inside the pen.
    #[test]
    fn huge_coordinates_draw_nothing_and_numbers_stay_short() {
        let m = test_measurer();
        // (attributes, most output bytes): the two 3,000-byte labels and no
        // glyphs, or one glyph; 1,000 glyphs drawn would be MBs.
        for (attrs, max_bytes) in [
            ("x=\"1e300\" y=\"1e300\"", 6_500),
            ("x=\"1e39\"", 6_500),
            ("dx=\"2e7 1e7 1e7 1e7\"", 6_500),
            ("y=\"-1e8\"", 6_500),
            // Only the first glyph is near the origin.
            ("style=\"letter-spacing:1e300px\"", 25_000),
            ("x=\"9999000\" style=\"letter-spacing:1e6px\"", 25_000),
            // Overflows to infinity: an invalid list, ignored (x = 0).
            ("x=\"1e999\"", usize::MAX),
        ] {
            let svg = wrap(&format!("<text {attrs}>{}</text>", "鬱".repeat(1000)));
            let out = outline_text(&svg, &m).unwrap();
            assert!(out.len() <= max_bytes, "{attrs}: {} bytes", out.len());
            let longest = out
                .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                .map(str::len)
                .max()
                .unwrap_or(0);
            assert!(longest <= 12, "{attrs}: a {longest}-byte number");
        }
        // In range, numbers are short too.
        let svg = wrap("<text x=\"9999900\" y=\"-9999900\">鬱</text>");
        let out = outline_text(&svg, &m).unwrap();
        assert!(out.contains("<path"));
        assert!(!out.contains("e+") && !out.contains("e-"));
    }

    /// The byte budget is checked as path data is written: a flood of big,
    /// dense glyphs stops at the budget instead of after a whole <text>.
    #[test]
    fn path_bytes_are_budgeted_inside_the_pen() {
        let m = test_measurer();
        let mut budget = Budget::default();
        budget.bytes.left = 50_000;
        let svg = wrap(&format!(
            "<text font-size=\"9000\" font-family=\"Noto Sans JP\">{}</text>",
            "鬱".repeat(200)
        ));
        let doc = Document::parse(&svg).unwrap();
        let text = doc.descendants().find(|n| is_named(*n, "text")).unwrap();
        let err = convert_text(text, &Style::default(), &svg, &m, &mut budget).unwrap_err();
        assert!(matches!(err, GfxError::InvalidSvg { .. }));
        assert!(budget.bytes.over);
    }

    /// Review of #27, finding 2: ancestor styles were parsed again for every
    /// <text>. Each element's style is parsed once now, empty texts shape
    /// nothing, and the number of texts is capped.
    #[test]
    fn styles_are_parsed_once_and_texts_are_capped() {
        let m = test_measurer();
        let big = format!("font-size:12px;{}", "a:b;".repeat(16_000));
        let svg = wrap(&format!(
            "<g style=\"{big}\">{}</g>",
            "<text/>".repeat(5_000)
        ));
        let out = outline_text(&svg, &m).unwrap();
        assert_eq!(out.matches("<g data-surimono-text=\"\"").count(), 5_000);
        let svg = wrap(&"<text/>".repeat(MAX_OUTLINE_TEXTS + 1));
        assert!(is_too_much(outline_text(&svg, &m)));
        // Styles computed top down reach the text through any depth: the
        // same glyphs as the style set on the text itself.
        let d = |svg: &str| {
            let out = outline_text(&wrap(svg), &m).unwrap();
            let start = out.find(" d=\"").unwrap();
            out[start..].split('"').nth(1).unwrap().to_string()
        };
        let nested = d(
            "<g style=\"font:bold 20px 'Noto Sans'\"><g><g letter-spacing=\"2\" text-anchor=\"end\"><text x=\"50\">AV</text></g></g></g>",
        );
        let direct = d(
            "<text x=\"50\" style=\"font:bold 20px 'Noto Sans'\" letter-spacing=\"2\" text-anchor=\"end\">AV</text>",
        );
        assert_eq!(nested, direct);
        let sibling = d(
            "<g font-size=\"40\"><rect/></g><text x=\"50\" style=\"font:bold 20px 'Noto Sans'\" letter-spacing=\"2\" text-anchor=\"end\">AV</text>",
        );
        assert_eq!(sibling, direct, "a closed sibling's style doesn't leak");
    }

    #[test]
    fn numbers_are_fixed_precision() {
        let mut s = String::new();
        for v in [0.0, -0.0001, 1.0, 1.5, -2.25, 1.23456, 12.005, 1e-9, -7.999] {
            push_number(&mut s, v);
            s.push(' ');
        }
        assert_eq!(s, "0 0 1 1.5 -2.25 1.23 12.01 0 -8 ");
    }
}
