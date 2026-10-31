//! Canvas `measureText` from Parley 0.6, the shaper fulgur prints with.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use parley::fontique::{Attributes, QueryFamily, QueryStatus, SourceCache};
use parley::{
    FontContext, FontFamily, FontStack, FontStyle, FontWeight, FontWidth, Layout, LayoutContext,
    PositionedLayoutItem, StyleProperty,
};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{GlyphId, MetadataProvider};

use crate::GfxError;
use crate::css_font::{FontSpec, Slant};
use crate::fonts::FontSet;

/// Longest text, in bytes, that [`TextMeasurer::measure`] accepts. Shaping is
/// not interruptible, so this bounds the time one call can take.
pub const MAX_TEXT_BYTES: usize = 256 * 1024;

/// Longest `css_font`, in bytes, that is parsed at all. A longer string is
/// treated as invalid and means canvas's default, `10px sans-serif`: no real
/// font shorthand comes close, and this bounds what a hostile script can make
/// the host parse and keep.
pub(crate) const MAX_CSS_FONT_BYTES: usize = 1024;
/// Canvas's default font, used for a `css_font` that is too long or invalid.
const CANVAS_DEFAULT_FONT: &str = "10px sans-serif";

/// Longest text or font string, in bytes, that is cached. Chart labels and
/// font strings are short; longer inputs are measured but never kept, so a
/// hostile script cannot fill the host's memory through the caches.
const MAX_CACHED_KEY_BYTES: usize = 256;
/// Total key bytes the result and font caches of one workspace may hold
/// before both are cleared.
const MAX_CACHED_KEY_BYTES_TOTAL: usize = 256 * 1024;
/// Cached measurements per workspace before the cache is cleared.
const MAX_CACHED_RESULTS: usize = 8192;
/// Cached parsed font strings per workspace before the cache is cleared.
const MAX_CACHED_FONTS: usize = 256;
/// Cached glyph bounds per workspace before the cache is cleared.
const MAX_CACHED_GLYPHS: usize = 16_384;
/// Idle workspaces kept for reuse. A workspace is per concurrent call, so this
/// covers up to this many threads measuring at once; with more, the extra
/// workspaces (and their caches) are dropped after each call, which costs
/// speed, never memory.
const MAX_IDLE_WORKSPACES: usize = 8;

/// After laying out more than this many bytes of text, a workspace gets
/// fresh Parley layout buffers before it goes back to the idle list: they
/// keep the capacity of the largest text they held (about 17 MB after one
/// 256 KiB text), and idle workspaces live as long as the measurer.
const LARGE_TEXT_BYTES: usize = 4 * 1024;

/// The result of measuring text, as canvas `TextMetrics` defines it.
///
/// All values are CSS px, for `textAlign = "start"`, `textBaseline =
/// "alphabetic"` and left-to-right direction. Serializes with the canvas
/// property names (`width`, `actualBoundingBoxLeft`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMetrics {
    /// Advance width of the text.
    pub width: f64,
    /// Distance from the alignment point to the left of the ink bounds
    /// (positive means the ink extends left of the start).
    pub actual_bounding_box_left: f64,
    /// Distance from the alignment point to the right of the ink bounds.
    pub actual_bounding_box_right: f64,
    /// Distance from the baseline to the top of the ink bounds.
    pub actual_bounding_box_ascent: f64,
    /// Distance from the baseline to the bottom of the ink bounds.
    pub actual_bounding_box_descent: f64,
    /// Ascent of the first available font in the family list.
    pub font_bounding_box_ascent: f64,
    /// Descent of the first available font in the family list.
    pub font_bounding_box_descent: f64,
}

/// Measures text with the registered fonts. Cheap to clone; clones share the
/// fonts and caches. Thread-safe: concurrent calls use separate workspaces.
///
/// Results are deterministic: the same fonts, text and font string give the
/// same bits, in any process, whatever was measured before.
#[derive(Clone)]
pub struct TextMeasurer {
    shared: Arc<Shared>,
}

struct Shared {
    fonts: FontSet,
    idle: Mutex<Vec<Workspace>>,
}

impl fmt::Debug for TextMeasurer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextMeasurer")
            .field("fonts", &self.shared.fonts)
            .finish_non_exhaustive()
    }
}

impl TextMeasurer {
    /// A measurer over these fonts.
    pub fn new(fonts: FontSet) -> Self {
        TextMeasurer {
            shared: Arc::new(Shared {
                fonts,
                idle: Mutex::new(Vec::new()),
            }),
        }
    }

    /// The fonts this measurer uses.
    pub fn fonts(&self) -> &FontSet {
        &self.shared.fonts
    }

    /// Measures `text` in `css_font`, a CSS `font` shorthand such as
    /// `"bold 14px 'Noto Sans JP', sans-serif"`.
    ///
    /// As canvas does, ASCII whitespace in `text` counts as spaces, and a
    /// `css_font` that does not parse means `10px sans-serif`, as does one
    /// longer than 1 KiB. Characters the listed families lack fall back to the
    /// other registered families.
    pub fn measure(&self, text: &str, css_font: &str) -> Result<TextMetrics, GfxError> {
        if text.len() > MAX_TEXT_BYTES {
            return Err(GfxError::TextTooLong {
                len: text.len(),
                max: MAX_TEXT_BYTES,
            });
        }
        if self.shared.fonts.face_count() == 0 {
            return Err(GfxError::NoFonts);
        }
        let css_font = if css_font.len() > MAX_CSS_FONT_BYTES {
            CANVAS_DEFAULT_FONT
        } else {
            css_font
        };
        let metrics = self.with_workspace(|ws, fonts| ws.measure(fonts, text, css_font));
        Ok(metrics)
    }

    /// Runs `f` with an idle workspace (or a new one), then returns the
    /// workspace to the idle list. The workspace is built outside the lock.
    pub(crate) fn with_workspace<R>(&self, f: impl FnOnce(&mut Workspace, &FontSet) -> R) -> R {
        let idle = self.idle().pop();
        let mut ws = idle.unwrap_or_else(|| Workspace::new(&self.shared.fonts));
        let out = f(&mut ws, &self.shared.fonts);
        ws.release_large_buffers();
        let mut idle = self.idle();
        if idle.len() < MAX_IDLE_WORKSPACES {
            idle.push(ws);
        }
        out
    }

    /// Key bytes actually held by the caches of every idle workspace, and the
    /// largest count of any one workspace, for tests of the cache bounds.
    #[cfg(test)]
    fn cached_key_bytes(&self) -> Vec<usize> {
        self.idle()
            .iter()
            .map(|ws| {
                let fonts: usize = ws.fonts.keys().map(String::len).sum();
                let results: usize = ws
                    .results
                    .iter()
                    .map(|(k, r)| k.len() + r.keys().map(String::len).sum::<usize>())
                    .sum();
                assert_eq!(fonts + results, ws.key_bytes, "key byte count drifted");
                fonts + results
            })
            .collect()
    }

    fn idle(&self) -> MutexGuard<'_, Vec<Workspace>> {
        // A panic while the lock was held can only have happened in Vec::push
        // or pop; the list is still valid, so recover it.
        self.shared
            .idle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A font string resolved against the font set.
struct ResolvedFont {
    size: f32,
    weight: FontWeight,
    style: FontStyle,
    width: FontWidth,
    stack: Vec<FontFamily<'static>>,
    /// Rounded ascent and descent of the primary font.
    font_ascent: f64,
    font_descent: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    blob: u64,
    index: u32,
    glyph: u32,
    size_bits: u32,
    skew_bits: u32,
}

/// Glyph ink bounds in whole px, y up: floor(xmin), floor(ymin), ceil(xmax), ceil(ymax).
type PixelBounds = [f64; 4];

/// Per-thread scratch state: Parley contexts and caches.
///
/// Every cache is bounded: keys longer than [`MAX_CACHED_KEY_BYTES`] are never
/// stored, and the result and font caches are cleared once they hold
/// [`MAX_CACHED_KEY_BYTES_TOTAL`] key bytes or their entry limit.
pub(crate) struct Workspace {
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,
    layout: Layout<()>,
    fonts: HashMap<String, Arc<ResolvedFont>>,
    results: HashMap<String, HashMap<String, TextMetrics>>,
    result_count: usize,
    /// Bytes of every key in `fonts` and `results` (outer and inner).
    key_bytes: usize,
    glyphs: HashMap<GlyphKey, Option<PixelBounds>>,
    /// How often the buffers were replaced (for tests).
    #[cfg(test)]
    buffer_resets: usize,
}

impl Workspace {
    fn new(fonts: &FontSet) -> Self {
        Workspace {
            font_cx: FontContext {
                collection: fonts.collection.clone(),
                source_cache: SourceCache::default(),
            },
            layout_cx: LayoutContext::new(),
            layout: Layout::new(),
            fonts: HashMap::new(),
            results: HashMap::new(),
            result_count: 0,
            key_bytes: 0,
            glyphs: HashMap::new(),
            #[cfg(test)]
            buffer_resets: 0,
        }
    }

    /// Replaces the layout buffers if the text the layout holds (the last
    /// one laid out) is large, so an idle workspace keeps only label-sized
    /// buffers. Every call lays out at most one text, whatever it measures
    /// or shapes, so this needs no bookkeeping in the callers.
    fn release_large_buffers(&mut self) {
        let laid_out = self
            .layout
            .lines()
            .last()
            .map_or(0, |line| line.text_range().end);
        if laid_out > LARGE_TEXT_BYTES {
            self.layout = Layout::new();
            self.layout_cx = LayoutContext::new();
            #[cfg(test)]
            {
                self.buffer_resets += 1;
            }
        }
    }

    /// Empties the result and font caches.
    fn clear_caches(&mut self) {
        self.results.clear();
        self.fonts.clear();
        self.result_count = 0;
        self.key_bytes = 0;
    }

    /// Key bytes that caching a result for this pair adds.
    fn result_cost(&self, css_font: &str, text: &str) -> usize {
        let outer = if self.results.contains_key(css_font) {
            0
        } else {
            css_font.len()
        };
        text.len() + outer
    }

    fn measure(&mut self, fonts: &FontSet, text: &str, css_font: &str) -> TextMetrics {
        if let Some(m) = self.results.get(css_font).and_then(|r| r.get(text)) {
            return *m;
        }
        let font = self.resolve(fonts, css_font);
        let metrics = self.measure_uncached(&font, text);
        if text.len() > MAX_CACHED_KEY_BYTES || css_font.len() > MAX_CACHED_KEY_BYTES {
            return metrics;
        }
        if self.result_count >= MAX_CACHED_RESULTS
            || self.key_bytes + self.result_cost(css_font, text) > MAX_CACHED_KEY_BYTES_TOTAL
        {
            self.clear_caches();
        }
        self.key_bytes += self.result_cost(css_font, text);
        self.results
            .entry(css_font.to_string())
            .or_default()
            .insert(text.to_string(), metrics);
        self.result_count += 1;
        metrics
    }

    fn resolve(&mut self, fonts: &FontSet, css_font: &str) -> Arc<ResolvedFont> {
        if let Some(f) = self.fonts.get(css_font) {
            return f.clone();
        }
        let spec = FontSpec::parse(css_font).unwrap_or_else(FontSpec::canvas_default);
        let stack: Vec<FontFamily<'static>> = fonts
            .resolve_stack(&spec.families)
            .into_iter()
            .map(|name| FontFamily::Named(Cow::Owned(name)))
            .collect();
        let weight = FontWeight::new(spec.weight);
        let style = match spec.slant {
            Slant::Normal => FontStyle::Normal,
            Slant::Italic => FontStyle::Italic,
            Slant::Oblique(angle) => FontStyle::Oblique(angle),
        };
        let width = FontWidth::from_ratio(spec.width);
        let (font_ascent, font_descent) =
            self.primary_metrics(&stack, Attributes::new(width, style, weight), spec.size_px);
        let resolved = Arc::new(ResolvedFont {
            size: spec.size_px,
            weight,
            style,
            width,
            stack,
            font_ascent,
            font_descent,
        });
        if css_font.len() <= MAX_CACHED_KEY_BYTES {
            if self.fonts.len() >= MAX_CACHED_FONTS
                || self.key_bytes + css_font.len() > MAX_CACHED_KEY_BYTES_TOTAL
            {
                self.clear_caches();
            }
            self.key_bytes += css_font.len();
            self.fonts.insert(css_font.to_string(), resolved.clone());
        }
        resolved
    }

    /// Ascent and descent of the first family's best face, rounded to whole
    /// px as Chrome (and Parley's quantized line layout) does.
    fn primary_metrics(
        &mut self,
        stack: &[FontFamily<'static>],
        attrs: Attributes,
        size: f32,
    ) -> (f64, f64) {
        let Some(FontFamily::Named(first)) = stack.first() else {
            return (0.0, 0.0);
        };
        let mut found = None;
        {
            let mut query = self
                .font_cx
                .collection
                .query(&mut self.font_cx.source_cache);
            query.set_families([QueryFamily::Named(first)]);
            query.set_attributes(attrs);
            query.matches_with(|font| {
                found = Some((font.blob.clone(), font.index));
                QueryStatus::Stop
            });
        }
        let Some((blob, index)) = found else {
            return (0.0, 0.0);
        };
        let Ok(font) = skrifa::FontRef::from_index(blob.as_ref(), index) else {
            return (0.0, 0.0);
        };
        let m = font.metrics(Size::new(size), LocationRef::default());
        (round_px(m.ascent as f64), round_px(-m.descent as f64))
    }

    fn measure_uncached(&mut self, font: &ResolvedFont, text: &str) -> TextMetrics {
        // Canvas: "replace all ASCII whitespace in text with U+0020 SPACE".
        let text: Cow<'_, str> = if text.contains(['\t', '\n', '\u{c}', '\r']) {
            Cow::Owned(text.replace(['\t', '\n', '\u{c}', '\r'], " "))
        } else {
            Cow::Borrowed(text)
        };
        let mut out = TextMetrics {
            font_bounding_box_ascent: font.font_ascent,
            font_bounding_box_descent: font.font_descent,
            ..TextMetrics::default()
        };
        if text.is_empty() {
            return out;
        }

        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, &text, 1.0, false);
        builder.push_default(StyleProperty::FontStack(FontStack::List(Cow::Borrowed(
            &font.stack,
        ))));
        builder.push_default(StyleProperty::FontSize(font.size));
        builder.push_default(StyleProperty::FontWeight(font.weight));
        builder.push_default(StyleProperty::FontStyle(font.style));
        builder.push_default(StyleProperty::FontWidth(font.width));
        builder.build_into(&mut self.layout, &text);
        self.layout.break_all_lines(None);

        // Ink bounds: per glyph, whole-pixel bounds placed at the glyph's
        // (fractional) position, as Chrome computes actualBoundingBox*.
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut ascent = f64::NEG_INFINITY;
        let mut descent = f64::NEG_INFINITY;
        let mut line_start = 0.0f64;
        let glyphs = &mut self.glyphs;
        for line in self.layout.lines() {
            let mut line_width = 0.0f64;
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                line_width += run.advance() as f64;
                let r = run.run();
                let blob = &r.font().data;
                let index = r.font().index;
                let size = r.font_size();
                let skew = r
                    .synthesis()
                    .skew()
                    .map(|deg| (deg as f64).to_radians().tan())
                    .unwrap_or(0.0);
                let baseline = run.baseline() as f64;
                for g in run.positioned_glyphs() {
                    let key = GlyphKey {
                        blob: blob.id(),
                        index,
                        glyph: g.id,
                        size_bits: size.to_bits(),
                        skew_bits: (skew as f32).to_bits(),
                    };
                    if glyphs.len() >= MAX_CACHED_GLYPHS && !glyphs.contains_key(&key) {
                        glyphs.clear();
                    }
                    let bounds = *glyphs
                        .entry(key)
                        .or_insert_with(|| glyph_bounds(blob.as_ref(), index, g.id, size, skew));
                    let Some([x0, y0, x1, y1]) = bounds else {
                        continue;
                    };
                    let x = line_start + g.x as f64;
                    // Shaping offsets move the glyph; y grows downwards here.
                    let dy = g.y as f64 - baseline;
                    min_x = min_x.min(x + x0);
                    max_x = max_x.max(x + x1);
                    ascent = ascent.max(y1 - dy);
                    descent = descent.max(-y0 + dy);
                }
            }
            line_start += line_width;
        }
        out.width = line_start;
        if min_x.is_finite() {
            out.actual_bounding_box_left = -min_x + 0.0;
            out.actual_bounding_box_right = max_x + 0.0;
            out.actual_bounding_box_ascent = ascent + 0.0;
            out.actual_bounding_box_descent = descent + 0.0;
        }
        out
    }
}

/// Rounds half up, like Skia's `SkScalarRoundToScalar`.
fn round_px(v: f64) -> f64 {
    (v + 0.5).floor() + 0.0
}

/// Control-box bounds of a glyph outline at `size` px, with a faux-italic
/// `skew` (x += skew * y), rounded outwards to whole px. `None` for glyphs
/// without ink, such as spaces.
fn glyph_bounds(data: &[u8], index: u32, glyph: u32, size: f32, skew: f64) -> Option<PixelBounds> {
    let font = skrifa::FontRef::from_index(data, index).ok()?;
    let outlines = font.outline_glyphs();
    let outline = outlines.get(GlyphId::new(glyph))?;
    let mut pen = BoundsPen { skew, bounds: None };
    outline
        .draw(
            DrawSettings::unhinted(Size::new(size), LocationRef::default()),
            &mut pen,
        )
        .ok()?;
    let [x0, y0, x1, y1] = pen.bounds?;
    Some([x0.floor(), y0.floor(), x1.ceil(), y1.ceil()])
}

/// Collects the bounding box of every point, on-curve or control.
struct BoundsPen {
    skew: f64,
    bounds: Option<[f64; 4]>,
}

impl BoundsPen {
    fn point(&mut self, x: f32, y: f32) {
        let (y, x) = (y as f64, x as f64 + self.skew * y as f64);
        self.bounds = Some(match self.bounds {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    }
}

impl OutlinePen for BoundsPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.point(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.point(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.point(cx0, cy0);
        self.point(x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.point(cx0, cy0);
        self.point(cx1, cy1);
        self.point(x, y);
    }
    fn close(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_matches_skia() {
        assert_eq!(round_px(12.83), 13.0);
        assert_eq!(round_px(3.516), 4.0);
        assert_eq!(round_px(2.5), 3.0);
        assert_eq!(round_px(0.293), 0.0);
        assert_eq!(round_px(-0.2).to_bits(), 0.0f64.to_bits());
    }

    /// Review of #3: after one large measure, the idle workspace holds fresh
    /// layout buffers; label-sized measures keep theirs.
    #[test]
    fn large_texts_leave_no_large_buffers_behind() {
        let mut fonts = FontSet::new();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/fonts/");
        fonts
            .add_font_bytes(std::fs::read(format!("{dir}NotoSans-Regular.ttf")).unwrap())
            .unwrap();
        let m = TextMeasurer::new(fonts);
        let resets = |m: &TextMeasurer| m.idle().iter().map(|w| w.buffer_resets).sum::<usize>();
        m.measure("Revenue 2026", "12px sans-serif").unwrap();
        m.measure(&"x".repeat(LARGE_TEXT_BYTES), "12px sans-serif")
            .unwrap();
        assert_eq!(resets(&m), 0, "label-sized texts keep the buffers");
        let big = "Sales 2026 ".repeat(MAX_TEXT_BYTES / 11);
        let a = m.measure(&big, "12px sans-serif").unwrap();
        assert_eq!(resets(&m), 1);
        let ws = m.idle();
        assert_eq!(ws.len(), 1);
        assert_eq!(ws[0].layout.len(), 0, "a fresh layout");
        drop(ws);
        // Measuring again in the fresh buffers gives the same result.
        assert_eq!(m.measure(&big, "12px sans-serif").unwrap(), a);
        assert_eq!(
            m.measure("Revenue 2026", "12px sans-serif").unwrap(),
            m.measure("Revenue 2026", "12px sans-serif").unwrap()
        );
    }

    #[test]
    fn empty_font_set_is_an_error() {
        let m = TextMeasurer::new(FontSet::new());
        assert_eq!(m.measure("x", "12px sans-serif"), Err(GfxError::NoFonts));
    }

    #[test]
    fn too_long_text_is_an_error() {
        let m = TextMeasurer::new(FontSet::new());
        let text = "a".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(
            m.measure(&text, "12px sans-serif"),
            Err(GfxError::TextTooLong {
                len: MAX_TEXT_BYTES + 1,
                max: MAX_TEXT_BYTES
            })
        );
    }

    fn noto_sans() -> TextMeasurer {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/fonts/NotoSans-Regular.ttf"
        );
        let mut fonts = FontSet::new();
        fonts
            .add_font_bytes(std::fs::read(path).expect("fixtures/fonts/NotoSans-Regular.ttf"))
            .unwrap();
        TextMeasurer::new(fonts)
    }

    /// A hostile script measuring long, unique texts and font strings must not
    /// grow the host: the caches stay under their byte budget.
    #[test]
    fn caches_stay_under_byte_budget_with_hostile_input() {
        let m = noto_sans();
        // Long unique texts (just under MAX_TEXT_BYTES): measured, never cached.
        for i in 0..8 {
            let text = format!("{i:08}{}", "x".repeat(MAX_TEXT_BYTES - 16));
            m.measure(&text, "12px 'Noto Sans'").unwrap();
        }
        // Huge unique font strings: invalid, never parsed or cached.
        for i in 0..50 {
            let font = format!("12px f{i}{}", "y".repeat(1 << 20));
            m.measure("label", &font).unwrap();
        }
        // Font strings just under the parse limit: parsed, never cached.
        for i in 0..200 {
            let font = format!("12px f{i:04}{}", "z".repeat(MAX_CSS_FONT_BYTES - 20));
            m.measure("label", &font).unwrap();
        }
        // Many short, unique keys at the cache limit: cached, then cleared.
        for i in 0..6000 {
            let font = format!("{}px f{i}{}", 8 + i % 7, "w".repeat(200));
            let text = format!("{i}{}", "v".repeat(240));
            m.measure(&text, &font).unwrap();
        }
        let held = m.cached_key_bytes();
        assert_eq!(held.len(), 1, "one thread uses one workspace");
        for bytes in held {
            assert!(
                bytes <= MAX_CACHED_KEY_BYTES_TOTAL,
                "{bytes} key bytes cached"
            );
        }
    }

    #[test]
    fn overlong_font_string_means_canvas_default() {
        let m = noto_sans();
        let long = format!("bold 30px 'Noto Sans'{}", " ".repeat(MAX_CSS_FONT_BYTES));
        let default = m.measure("Revenue", "10px sans-serif").unwrap();
        assert_eq!(m.measure("Revenue", &long).unwrap(), default);
        let parsed = m.measure("Revenue", long.trim_end()).unwrap();
        assert_ne!(
            parsed, default,
            "the same font string, short enough, parses"
        );
    }

    #[test]
    fn measurer_is_send_sync_clone() {
        fn check<T: Send + Sync + Clone + 'static>() {}
        check::<TextMeasurer>();
        check::<FontSet>();
    }

    #[test]
    fn metrics_serialize_with_canvas_names() {
        let json = serde_json::to_string(&TextMetrics {
            width: 1.5,
            ..TextMetrics::default()
        })
        .unwrap();
        assert_eq!(
            json,
            "{\"width\":1.5,\"actualBoundingBoxLeft\":0.0,\"actualBoundingBoxRight\":0.0,\
             \"actualBoundingBoxAscent\":0.0,\"actualBoundingBoxDescent\":0.0,\
             \"fontBoundingBoxAscent\":0.0,\"fontBoundingBoxDescent\":0.0}"
        );
    }
}
