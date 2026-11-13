//! Options for a [`Renderer`](crate::Renderer).

use std::fmt;
use std::time::Duration;

use crate::{ErrorPolicy, ResourceLoader};

/// Resource limits for every script a renderer runs, applied to each island
/// separately, and to a page's scripts as one
/// ([`Renderer::render_page`](crate::Renderer::render_page)).
///
/// A script that goes over a limit stops with [`Error::MemoryLimit`],
/// [`Error::TimeLimit`] or [`Error::StackOverflow`](crate::Error::StackOverflow);
/// it never crashes or hangs the process.
///
/// The limits apply **per island**: every island gets a fresh JavaScript heap of
/// `memory_bytes` and one deadline of `time` covering loading its renderer and
/// drawing it, so a document with N islands can take up to N × `time`.
/// [`Options::max_islands`] caps N (1,000 by default), and
/// [`Renderer::render_islands_cancellable`](crate::Renderer::render_islands_cancellable)
/// stops a render from outside.
///
/// Build limits from the defaults, so a limit added in a later release keeps
/// its default in your code:
///
/// ```
/// use std::time::Duration;
/// use surimono::Limits;
///
/// let limits = Limits { time: Duration::from_millis(500), ..Limits::default() };
/// assert_eq!(limits.memory_bytes, Limits::DEFAULT_MEMORY_BYTES);
/// ```
///
/// [`Error::MemoryLimit`]: crate::Error::MemoryLimit
/// [`Error::TimeLimit`]: crate::Error::TimeLimit
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Limits {
    /// Most bytes the JavaScript heap may use. At least 1 MiB.
    pub memory_bytes: usize,
    /// Longest one island may run: one deadline per island covering loading
    /// the renderer's bundle into a fresh runtime and drawing the island. A
    /// page's scripts share one such limit, its parse has another, and
    /// everything after its scripts (serializing, loading stylesheets, the
    /// checks for a printer) a third.
    /// Must be more than zero.
    pub time: Duration,
    /// Most bytes of native stack the JavaScript engine may use, from 64 KiB to 16 MiB.
    pub stack_bytes: usize,
    /// Most bytes of HTML [`render_page`](crate::Renderer::render_page) may
    /// give back, and of a page's text while it runs (up to the document's
    /// own 256 MiB): past either, [`Error::PageDomLimit`](crate::Error::PageDomLimit).
    /// Escaping can make a page's output many times what it holds, and a
    /// printer would have to read all of it. At least 1 MiB.
    pub output_bytes: usize,
}

impl Limits {
    /// Default heap limit: 64 MiB.
    pub const DEFAULT_MEMORY_BYTES: usize = 64 * 1024 * 1024;
    /// Default time limit: 2 seconds per island.
    pub const DEFAULT_TIME: Duration = Duration::from_secs(2);
    /// Default stack limit: 512 KiB.
    pub const DEFAULT_STACK_BYTES: usize = 512 * 1024;
    /// Default output limit: 64 MiB.
    pub const DEFAULT_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: Self::DEFAULT_MEMORY_BYTES,
            time: Self::DEFAULT_TIME,
            stack_bytes: Self::DEFAULT_STACK_BYTES,
            output_bytes: Self::DEFAULT_OUTPUT_BYTES,
        }
    }
}

/// A JavaScript bundle that renders islands: any script defining a function
/// that takes the island's JSON and returns an SVG string.
#[derive(Clone)]
pub(crate) struct Bundle {
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) entry: String,
}

impl fmt::Debug for Bundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bundle")
            .field("name", &self.name)
            .field("source_bytes", &self.source.len())
            .field("entry", &self.entry)
            .finish()
    }
}

/// How a [`Renderer`](crate::Renderer) is set up: fonts, limits, the random
/// seed, the error policy and which renderers islands may use.
///
/// Every setting but the fonts has a default. A renderer needs at least one
/// font ([`font`](Self::font)): the files your PDF engine prints with, since
/// text is only ever measured and outlined with them.
///
/// ```
/// use surimono::{ErrorPolicy, Limits, Options, Renderer};
///
/// # let noto_sans = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/fonts/NotoSans-Regular.ttf"))
/// #     .unwrap_or_else(|e| panic!("{e}"));
/// let options = Options::new()
///     .font(noto_sans)
///     .seed(7)
///     .limits(Limits::default())
///     .error_policy(ErrorPolicy::Placeholder);
/// let renderer = Renderer::new(options)?;
/// # let _ = renderer;
/// # Ok::<(), surimono::Error>(())
/// ```
#[derive(Clone, Default)]
pub struct Options {
    pub(crate) fonts: Vec<Vec<u8>>,
    pub(crate) limits: Limits,
    pub(crate) seed: u64,
    pub(crate) error_policy: ErrorPolicy,
    pub(crate) renderers: Option<Vec<String>>,
    pub(crate) bundles: Vec<Bundle>,
    pub(crate) pool_size: Option<usize>,
    pub(crate) now_ms: Option<i64>,
    pub(crate) utc_offset_minutes: i32,
    pub(crate) text_mode: TextMode,
    pub(crate) max_islands: Option<usize>,
    pub(crate) layout_queries: LayoutQueries,
    pub(crate) keep_scripts: bool,
    /// [`printer_safe`](Self::printer_safe) is off.
    pub(crate) plain_output: bool,
    pub(crate) resource_loader: Option<ResourceLoader>,
}

/// What a page's script gets when it asks about layout
/// (`getBoundingClientRect`, `offsetWidth`, `getComputedStyle` and the like).
/// Nothing is laid out in this release, so no answer is the real one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum LayoutQueries {
    /// Sizes and positions are 0, rectangle lists are empty and
    /// `getComputedStyle(…).getPropertyValue()` is an empty string (its named
    /// properties are undefined), so a page that only reads them carries on,
    /// and may draw at size 0. The default.
    #[default]
    Zeros,
    /// The first query ends the render with
    /// [`Error::PageLayoutQuery`](crate::Error::PageLayoutQuery), even if the
    /// page catches the exception, for when you'd rather have no page than one
    /// built on measurements that aren't real.
    Throw,
}

/// What happens to text in the SVG a renderer returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TextMode {
    /// Every `<text>` becomes `<path>` outlines of its glyphs in your fonts,
    /// shaped exactly as it was measured, so any PDF engine prints what was
    /// measured whatever fonts it has. The text stays as an `aria-label`, but
    /// can't be selected or searched in the PDF. The default.
    #[default]
    Outlines,
    /// `<text>` stays text, with `font-family` mapped to your fonts. Only right
    /// for a PDF engine that sets inline SVG text in the fonts you passed:
    /// fulgur 0.40 uses system fonts there, so labels can differ from what was
    /// measured, or go missing.
    Keep,
}

/// The clock every render sees when [`Options::now_ms`] isn't set:
/// 2026-01-01T00:00:00Z.
pub const DEFAULT_NOW_MS: i64 = 1_767_225_600_000;

/// Most JavaScript runtimes a renderer keeps by default.
pub(crate) const DEFAULT_MAX_POOL_SIZE: usize = 4;

/// The default pool size: the number of CPUs, at most 4.
pub(crate) fn default_pool_size() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .clamp(1, DEFAULT_MAX_POOL_SIZE)
}

impl Options {
    /// Default options: default [`Limits`], seed 0, [`ErrorPolicy::Fail`],
    /// every built-in renderer, a pool sized to the machine, the clock at
    /// [`DEFAULT_NOW_MS`] in UTC, text as outlines, at most 1,000 islands per
    /// document, zeros for a page's layout queries, its scripts removed from
    /// the output, that output printer-safe, no resource loader, and no fonts
    /// yet: add at least one with
    /// [`font`](Self::font), or [`Renderer::new`](crate::Renderer::new) fails
    /// with [`Error::InvalidOptions`](crate::Error::InvalidOptions).
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a font file (TrueType, OpenType or a collection) for measuring and
    /// outlining text. At least one is required.
    ///
    /// Use the same font files your PDF engine prints with, so labels measured
    /// here fit there. surimono never reads system fonts.
    pub fn font(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.fonts.push(bytes.into());
        self
    }

    /// Sets the resource limits for every script.
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Sets the seed for `Math.random`. The same seed and input give the same output.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Sets what happens when an island fails. Default: [`ErrorPolicy::Fail`].
    pub fn error_policy(mut self, policy: ErrorPolicy) -> Self {
        self.error_policy = policy;
        self
    }

    /// Enables only the named renderers (built-in or added with
    /// [`bundle`](Self::bundle)). By default every renderer is enabled. An island
    /// naming a disabled renderer fails with
    /// [`Error::UnknownRenderer`](crate::Error::UnknownRenderer).
    pub fn renderers<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.renderers = Some(names.into_iter().map(Into::into).collect());
        self
    }

    /// Adds a renderer from your own JavaScript bundle.
    ///
    /// `source` is a classic script that defines a global function named
    /// `entry` (by convention `"entry"`; with a bundler, assign it to
    /// `globalThis`). For each island it is called as `entry(specJson, ctx)`:
    /// `specJson` is the island's spec as JSON **text** (call `JSON.parse`),
    /// and `ctx` has `width` and `height` in CSS pixels and
    /// `measureText(text, cssFont)`, which returns canvas-style metrics
    /// measured with your fonts. It returns an SVG string (or a Promise of
    /// one), which is sanitised before it goes into the document.
    ///
    /// Islands use it with `data-renderer="<name>"`. The bundle is compiled
    /// once and runs in the same sandbox, with the same limits, as the
    /// built-in renderers.
    ///
    /// ```
    /// use surimono::{Options, Renderer};
    ///
    /// # let font = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/fonts/NotoSans-Regular.ttf"))
    /// #     .unwrap_or_else(|e| panic!("{e}"));
    /// let renderer = Renderer::new(Options::new().font(font).bundle(
    ///     "badge",
    ///     "function entry(specJson, ctx) {
    ///        const spec = JSON.parse(specJson);
    ///        return `<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"${ctx.width}\" \
    ///                height=\"${ctx.height}\"><rect width=\"${spec.w}\" height=\"4\"/></svg>`;
    ///      }",
    ///     "entry",
    /// ))?;
    /// let html = r#"<script type="application/vnd.surimono+json" data-renderer="badge"
    ///   data-width="40" data-height="4">{"w": 30}</script>"#;
    /// assert!(renderer.render_islands(html)?.contains("<rect width=\"30\""));
    /// # Ok::<(), surimono::Error>(())
    /// ```
    pub fn bundle(
        mut self,
        name: impl Into<String>,
        source: impl Into<String>,
        entry: impl Into<String>,
    ) -> Self {
        self.bundles.push(Bundle {
            name: name.into(),
            source: source.into(),
            entry: entry.into(),
        });
        self
    }

    /// Sets how many JavaScript runtimes (each on its own thread) the renderer
    /// keeps for concurrent renders. Default: the number of CPUs, at most 4.
    /// At most 64.
    pub fn pool_size(mut self, size: usize) -> Self {
        self.pool_size = Some(size);
        self
    }

    /// Sets what `Date.now()` and `new Date()` return in every render, in
    /// milliseconds since the Unix epoch, for a fixed "today" in charts.
    /// Default: [`DEFAULT_NOW_MS`]. The host's clock is never read.
    pub fn now_ms(mut self, ms: i64) -> Self {
        self.now_ms = Some(ms);
        self
    }

    /// Sets what happens to text in renderers' SVG. Default:
    /// [`TextMode::Outlines`]; outlining needs at least one font.
    pub fn text_mode(mut self, mode: TextMode) -> Self {
        self.text_mode = mode;
        self
    }

    /// Sets the most island markers one document may have. A document with
    /// more fails with [`Error::TooManyIslands`](crate::Error::TooManyIslands)
    /// before any island runs, whatever the error policy. Each island has its
    /// own time limit, so this bounds how long one document can take: up to
    /// `max_islands` × [`Limits::time`]. Default: 1,000. Must be at least 1.
    pub fn max_islands(mut self, max: usize) -> Self {
        self.max_islands = Some(max);
        self
    }

    /// Sets what a page's script gets from layout queries. Default:
    /// [`LayoutQueries::Zeros`].
    pub fn layout_queries(mut self, queries: LayoutQueries) -> Self {
        self.layout_queries = queries;
        self
    }

    /// Keeps `<script>` elements in the HTML
    /// [`Renderer::render_page`](crate::Renderer::render_page) returns.
    /// Default: every `<script>` but island markers is removed, whether it
    /// ran or not, because the page has run and a later step mustn't run or
    /// print it again.
    pub fn keep_scripts(mut self, keep: bool) -> Self {
        self.keep_scripts = keep;
        self
    }

    /// Sets whether [`Renderer::render_page`](crate::Renderer::render_page)
    /// writes its output in forms a browserless printer prints the way Chrome
    /// prints the page. Default: on.
    ///
    /// fulgur 0.40 and 0.41 print some ordinary pages wrong although their DOM
    /// is right. For three of those gaps the page has an equivalent form that
    /// prints right, and `render_page` writes it:
    ///
    /// - a block-level `<svg>` keeps the size its `width` and `height`
    ///   attributes give it (fulgur ignores them), through a rule in a new
    ///   `<style>` at the start of `<head>` that ranks below all of the
    ///   page's CSS;
    /// - a border on one side (`border-bottom: 1px solid #ccc`) prints in its
    ///   colour, not the text's, through `border-top-color` in the same sheet;
    /// - a `<pre>` whose text ends in a newline isn't a line taller: the
    ///   newline, which a browser doesn't render, is left out.
    ///
    /// Each is made only where Chrome renders the rewritten page exactly as
    /// it renders the original, whatever the page's CSS: where that can't be
    /// told from the page, nothing is rewritten. So nothing is in a page with
    /// CSS that isn't in the page itself (a linked style sheet, an `@import`:
    /// load them with a [`ResourceLoader`] and they are), and nothing in one
    /// that keeps a script a browser would run while it prints: its
    /// `<script>` elements ([`keep_scripts`](Self::keep_scripts)), an event
    /// handler attribute (`onload`, `onerror`), an `<iframe srcdoc>`, a
    /// `javascript:` URL, or a frame or object that loads a document. Turn
    /// this off for output that is the page's DOM and nothing else.
    pub fn printer_safe(mut self, on: bool) -> Self {
        self.plain_output = !on;
        self
    }

    /// Lets a page's external scripts, frames and stylesheets load, through
    /// `loader` only. Default: none, so network and files are off. See
    /// [`ResourceLoader`].
    pub fn resource_loader(mut self, loader: ResourceLoader) -> Self {
        self.resource_loader = Some(loader);
        self
    }

    /// Sets local time's offset from UTC in minutes (540 for Japan), so dates
    /// on time axes show in that zone. From -720 to 840. Default: 0 (UTC).
    /// The host's time zone is never read.
    pub fn utc_offset_minutes(mut self, minutes: i32) -> Self {
        self.utc_offset_minutes = minutes;
        self
    }
}

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let font_bytes: Vec<usize> = self.fonts.iter().map(Vec::len).collect();
        f.debug_struct("Options")
            .field("font_bytes", &font_bytes)
            .field("limits", &self.limits)
            .field("seed", &self.seed)
            .field("error_policy", &self.error_policy)
            .field("renderers", &self.renderers)
            .field("bundles", &self.bundles)
            .field("pool_size", &self.pool_size)
            .field("now_ms", &self.now_ms)
            .field("utc_offset_minutes", &self.utc_offset_minutes)
            .field("text_mode", &self.text_mode)
            .field("max_islands", &self.max_islands)
            .field("layout_queries", &self.layout_queries)
            .field("keep_scripts", &self.keep_scripts)
            .field("printer_safe", &!self.plain_output)
            .field("resource_loader", &self.resource_loader)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let o = Options::new();
        assert!(o.fonts.is_empty());
        assert_eq!(o.limits, Limits::default());
        assert_eq!(o.seed, 0);
        assert_eq!(o.error_policy, ErrorPolicy::Fail);
        assert_eq!(o.renderers, None);
        assert!(o.bundles.is_empty());
        assert_eq!(o.pool_size, None);
        assert_eq!(o.max_islands, None);
        assert_eq!(o.text_mode, TextMode::Outlines);
        assert_eq!(o.layout_queries, LayoutQueries::Zeros);
        assert!(!o.keep_scripts);
        assert!(!o.plain_output);
        assert!(o.clone().printer_safe(false).plain_output);
        assert!(
            !o.clone()
                .printer_safe(false)
                .printer_safe(true)
                .plain_output
        );
        assert!(o.resource_loader.is_none());
        assert_eq!(Limits::default().memory_bytes, 64 * 1024 * 1024);
        assert_eq!(Limits::default().time, Duration::from_secs(2));
        assert_eq!(Limits::default().stack_bytes, 512 * 1024);
    }

    #[test]
    fn builder_sets_everything() {
        let o = Options::new()
            .font(vec![1, 2, 3])
            .font(&b"abcd"[..])
            .seed(9)
            .error_policy(ErrorPolicy::Placeholder)
            .renderers(["echarts", "mine"])
            .bundle("mine", "function f(){}", "f")
            .pool_size(3)
            .max_islands(20);
        assert_eq!(o.fonts, vec![vec![1, 2, 3], b"abcd".to_vec()]);
        assert_eq!(o.seed, 9);
        assert_eq!(o.error_policy, ErrorPolicy::Placeholder);
        assert_eq!(
            o.renderers,
            Some(vec!["echarts".to_owned(), "mine".to_owned()])
        );
        assert_eq!(o.bundles[0].name, "mine");
        assert_eq!(o.pool_size, Some(3));
        assert_eq!(o.max_islands, Some(20));
    }

    #[test]
    fn debug_does_not_dump_font_or_bundle_bytes() {
        let o = Options::new()
            .font(vec![0u8; 100_000])
            .bundle("b", "x".repeat(50_000), "f");
        let s = format!("{o:?}");
        assert!(s.len() < 500, "{s}");
        assert!(s.contains("100000") && s.contains("50000"), "{s}");
    }
}
