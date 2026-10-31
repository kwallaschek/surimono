//! The [`Renderer`]: options in, islands rendered.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, TryLockError};

use surimono_gfx::{FontSet, TextMeasurer};
use surimono_islands::{IslandRequest, IslandRunner};

use crate::Options;
use crate::backend;
use crate::error::Error;
use crate::prepass::{self, Document, ErrorPolicy};
use crate::svg::SanitizedSvg;

/// Renders a document's islands to SVG.
///
/// Build one per configuration and reuse it: it keeps a pool of JavaScript
/// runtimes, compiles each renderer's bundle once, and measures text with the
/// fonts you gave it. It is `Send + Sync`, so one renderer can serve many
/// threads at a time.
///
/// A renderer survives `fork` (Puma's `preload_app!` and `fork_worker`,
/// Unicorn, Resque): worker threads don't, so the first render in a forked
/// process starts a new pool and text measurer there, sharing the font data
/// with the parent. Nothing it needs in the child is a lock a parent thread
/// could have held at the moment of the fork.
///
/// ```
/// use surimono::{Options, Renderer};
///
/// # let font = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/fonts/NotoSans-Regular.ttf"))
/// #     .unwrap_or_else(|e| panic!("{e}"));
/// let renderer = Renderer::new(Options::new().font(font))?;
/// let html = "<h1>Q3 report</h1><p>No charts on this page.</p>";
/// assert_eq!(renderer.render_islands(html)?, html);
/// # Ok::<(), surimono::Error>(())
/// ```
pub struct Renderer {
    /// The options it was built with (fonts moved out), to start the pool
    /// again in a forked child.
    options: Options,
    /// The caller's fonts. Never mutated after `new` and holding no lock, so
    /// a forked child can clone it safely (the clone shares the font data).
    fonts: FontSet,
    /// Per-process state, see [`Slot`]: `[0]` is the process that built the
    /// renderer, then one slot per fork along this process's ancestry.
    slots: Box<[Slot; MAX_FORK_DEPTH]>,
    policy: ErrorPolicy,
    enabled: Vec<String>,
}

/// Fork generations one renderer can follow (a process forked from a process
/// forked from … the one that built it). Puma's `fork_worker` and Resque go
/// two or three deep.
const MAX_FORK_DEPTH: usize = 16;

/// The state one process renders with: its own runner (worker threads) and
/// text measurer (whose internal lock a parent thread may have held at fork
/// time, so a child never touches its parent's).
///
/// Finding this process's slot takes only atomic loads, never a lock: the
/// slot claimed last in this process's memory is its own if its pid matches,
/// and otherwise this process is a fork and claims the next free slot with a
/// compare-and-swap. A process forked from it sees that slot as claimed by
/// another pid and claims the one after. `state` is only ever written by the
/// process that claimed the slot, so a child never waits on a `OnceLock` a
/// parent thread was filling.
struct Slot {
    /// The pid that claimed this slot; 0 while free.
    pid: AtomicU32,
    state: OnceLock<Local>,
}

impl Slot {
    fn free() -> Self {
        Slot {
            pid: AtomicU32::new(0),
            state: OnceLock::new(),
        }
    }
}

/// One process's runner and measurer.
struct Local {
    /// Measures text for renderers and outlines it in their SVG.
    measurer: TextMeasurer,
    runner: IslandRunner,
}

/// Process state given up on in this process; see [`abandon`].
static ABANDONED: Mutex<Vec<Local>> = Mutex::new(Vec::new());

/// Gives up on state whose threads belong to another process: dropping it
/// would join threads that don't exist here, which never returns. It is kept
/// for the life of the process instead (its memory is the parent's, copied on
/// write), and stays reachable, so leak checkers don't report it. If the list
/// is locked, possibly by a thread that didn't survive a fork, it is leaked
/// rather than waited for.
fn abandon(local: Local) {
    match ABANDONED.try_lock() {
        Ok(mut list) => list.push(local),
        Err(TryLockError::Poisoned(p)) => p.into_inner().push(local),
        Err(TryLockError::WouldBlock) => {
            Box::leak(Box::new(local));
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let pid = std::process::id();
        let own = last_claimed(&self.slots[..]);
        for (i, slot) in self.slots.iter_mut().enumerate() {
            // Only the last claimed slot can be this process's own: an earlier
            // one with the same pid belonged to an ancestor whose pid was reused.
            if i == own && *slot.pid.get_mut() == pid {
                continue;
            }
            if let Some(local) = slot.state.take() {
                abandon(local);
            }
        }
    }
}

/// Index of the last slot claimed in this process's memory (slot 0 is claimed
/// by `Renderer::new`).
fn last_claimed(slots: &[Slot]) -> usize {
    slots
        .iter()
        .rposition(|s| s.pid.load(Ordering::Acquire) != 0)
        .unwrap_or(0)
}

/// Most islands one document may have by default; see [`Options::max_islands`].
pub(crate) const DEFAULT_MAX_ISLANDS: usize = 1000;

impl Renderer {
    /// Builds a renderer: loads the fonts, checks the limits, starts the
    /// runtime pool and compiles your bundles. Built-in renderers compile on
    /// first use.
    ///
    /// # Errors
    ///
    /// [`Error::Font`] for a font file that can't be read,
    /// [`Error::InvalidOptions`] for no fonts, limits out of range (including a
    /// zero time limit), a pool size or `max_islands` of 0 or an unknown name
    /// in [`Options::renderers`], and [`Error::Bundle`] for a bundle that can't
    /// be used.
    pub fn new(mut options: Options) -> Result<Self, Error> {
        if options.limits.time.is_zero() {
            return Err(Error::InvalidOptions(
                "limits.time is zero; every island would fail".into(),
            ));
        }
        if options.pool_size == Some(0) {
            return Err(Error::InvalidOptions("pool_size must be at least 1".into()));
        }
        if options.max_islands == Some(0) {
            return Err(Error::InvalidOptions(
                "max_islands must be at least 1".into(),
            ));
        }
        if options.fonts.is_empty() {
            return Err(Error::InvalidOptions(
                "no fonts were given: pass the font files your PDF engine prints with \
                 (Options::font); surimono measures and outlines text only with them, never \
                 with system fonts"
                    .into(),
            ));
        }
        let fonts = backend::font_set(std::mem::take(&mut options.fonts))?;
        let mut available = backend::builtin_renderers();
        for b in &options.bundles {
            backend::check_bundle(&b.name, &b.entry, &available).map_err(|message| {
                Error::Bundle {
                    name: b.name.clone(),
                    message,
                }
            })?;
            available.push(b.name.clone());
        }
        let local = start_local(&options, &fonts)?;
        let enabled = match &options.renderers {
            None => available,
            Some(wanted) => {
                if let Some(unknown) = wanted.iter().find(|w| !available.contains(w)) {
                    return Err(Error::InvalidOptions(format!(
                        "renderer {unknown:?} doesn't exist; available: {}",
                        available.join(", ")
                    )));
                }
                let mut enabled: Vec<String> = Vec::with_capacity(wanted.len());
                for w in wanted {
                    if !enabled.contains(w) {
                        enabled.push(w.clone());
                    }
                }
                enabled
            }
        };
        let slots: Box<[Slot; MAX_FORK_DEPTH]> = Box::new(std::array::from_fn(|_| Slot::free()));
        slots[0].pid.store(std::process::id(), Ordering::Release);
        if slots[0].state.set(local).is_err() {
            // A fresh OnceLock is empty; this can't happen.
            return Err(Error::InvalidOptions("the renderer could not start".into()));
        }
        Ok(Self {
            policy: options.error_policy,
            options,
            fonts,
            slots,
            enabled,
        })
    }

    /// This process's runner and measurer, started on first use in a forked
    /// child. Takes no lock on the way that a thread which didn't survive a
    /// fork could hold.
    fn local(&self) -> Result<&Local, Error> {
        let pid = std::process::id();
        loop {
            let last = last_claimed(&self.slots[..]);
            let slot = &self.slots[last];
            if slot.pid.load(Ordering::Acquire) == pid {
                if let Some(local) = slot.state.get() {
                    return Ok(local);
                }
                // Claimed by this process and not started yet. Threads of this
                // process may race here; the losers drop their own runner.
                let fresh = start_local(&self.options, &self.fonts)?;
                drop(slot.state.set(fresh));
                return slot.state.get().ok_or_else(|| {
                    Error::InvalidOptions("the renderer could not start in this process".into())
                });
            }
            // A fork: claim the next slot for this process, or pick up the one
            // another thread of this process just claimed.
            let Some(next) = self.slots.get(last + 1) else {
                return Err(Error::InvalidOptions(format!(
                    "this renderer was built {MAX_FORK_DEPTH} fork generations ago; \
                     build a new Renderer in this process"
                )));
            };
            let _ = next
                .pid
                .compare_exchange(0, pid, Ordering::AcqRel, Ordering::Acquire);
        }
    }

    /// Replaces every island marker in `html` with the SVG its renderer draws,
    /// leaving everything else byte for byte as it was.
    ///
    /// An island marker is a script element of type
    /// `application/vnd.surimono+json` holding the renderer's JSON spec:
    ///
    /// ```html
    /// <script type="application/vnd.surimono+json" id="sales" class="chart"
    ///         data-renderer="echarts" data-width="600" data-height="400">
    ///   { "xAxis": { "data": ["Q1", "Q2"] }, "series": [{ "type": "bar", "data": [3, 5] }] }
    /// </script>
    /// ```
    ///
    /// It becomes `<div class="surimono-island chart" id="sales"
    /// data-renderer="echarts"><svg …>…</svg></div>`. Without `data-width` and
    /// `data-height` an island is 600 × 400 CSS pixels. Put markers where a
    /// `div` is allowed, not inside `<p>`, `<span>` or `<a>`. Inside the JSON,
    /// write `<` as `<` (Rails' `to_json` does), so no `</script>` or
    /// `<!--` in a string can end or confuse the marker.
    ///
    /// Every renderer's SVG is sanitised before it goes in: no scripts, event
    /// handlers, external references, style sheets or markup that could end
    /// the wrapper or reach the rest of the page; ids get a per-island prefix;
    /// font families map to your fonts. SVG that can't be made safe fails the
    /// island with [`Error::Render`].
    ///
    /// Islands render in document order, one after another. Each has its own
    /// time limit ([`Limits::time`](crate::Limits::time)), so a document with
    /// N islands can take up to N × that; [`Options::max_islands`] caps N
    /// (1,000 by default), and [`render_islands_cancellable`](Self::render_islands_cancellable)
    /// lets you stop a render between islands. The same renderer, options and
    /// input always give the same bytes.
    ///
    /// # Errors
    ///
    /// With [`ErrorPolicy::Fail`] (the default), the first island that fails ends
    /// the render with its error: [`Error::InvalidIsland`],
    /// [`Error::UnknownRenderer`], [`Error::MemoryLimit`], [`Error::TimeLimit`],
    /// [`Error::StackOverflow`], [`Error::JavaScript`] or [`Error::Render`]. With
    /// [`ErrorPolicy::Placeholder`] failed islands become placeholders instead.
    /// Whatever the policy, [`Error::TooManyIslands`] if the document has more
    /// islands than [`Options::max_islands`] (before any of them runs), and
    /// [`Error::Html`] if the markup is too ambiguous to rewrite safely.
    pub fn render_islands(&self, html: &str) -> Result<String, Error> {
        self.render(html, None)
    }

    /// [`render_islands`](Self::render_islands) that stops early once `cancel`
    /// is true: it is checked before each island, and the islands not yet
    /// started are skipped with [`Error::Cancelled`] whatever the error
    /// policy. An island already running finishes or stops at its own time
    /// limit.
    ///
    /// Set it from another thread, a timer or a signal handler (storing to an
    /// `AtomicBool` is async-signal-safe). The Ruby gem sets it when the
    /// rendering thread is interrupted (`Timeout`, `Thread#raise`,
    /// `Thread#kill`, a signal).
    ///
    /// ```
    /// use std::sync::atomic::AtomicBool;
    /// use surimono::{Error, Options, Renderer};
    ///
    /// # let font = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/fonts/NotoSans-Regular.ttf"))
    /// #     .unwrap_or_else(|e| panic!("{e}"));
    /// let renderer = Renderer::new(Options::new().font(font))?;
    /// let html = r#"<script type="application/vnd.surimono+json" data-renderer="echarts">{}</script>"#;
    /// let cancel = AtomicBool::new(true);
    /// assert!(matches!(renderer.render_islands_cancellable(html, &cancel), Err(Error::Cancelled)));
    /// # Ok::<(), surimono::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// As [`render_islands`](Self::render_islands), plus [`Error::Cancelled`].
    pub fn render_islands_cancellable(
        &self,
        html: &str,
        cancel: &AtomicBool,
    ) -> Result<String, Error> {
        self.render(html, Some(cancel))
    }

    fn render(&self, html: &str, cancel: Option<&AtomicBool>) -> Result<String, Error> {
        let document = Document {
            policy: self.policy,
            max_islands: self.options.max_islands.unwrap_or(DEFAULT_MAX_ISLANDS),
            cancel,
        };
        // Documents without islands never need the runner.
        let mut local: Option<&Local> = None;
        prepass::rewrite_islands(html, &document, |island| {
            if !self.enabled.contains(&island.renderer) {
                return Err(Error::UnknownRenderer {
                    island: island.at.clone(),
                    enabled: self.enabled.clone(),
                });
            }
            let request = IslandRequest {
                renderer: &island.renderer,
                spec_json: &island.spec_json,
                width: island.width,
                height: island.height,
            };
            let local = match local {
                Some(l) => l,
                None => *local.insert(self.local()?),
            };
            let svg = local
                .runner
                .render(&request)
                .map_err(|e| backend::island_error(e, island.at.clone(), &self.enabled))?;
            SanitizedSvg::new(&svg, &island.at, &local.measurer, self.options.text_mode)
        })
    }

    /// The renderers islands may name: the built-in ones compiled into this
    /// build (sorted), then your bundles in the order added, or only those
    /// enabled with [`Options::renderers`], in that order.
    pub fn renderers(&self) -> &[String] {
        &self.enabled
    }
}

impl fmt::Debug for Renderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Renderer")
            .field("renderers", &self.enabled)
            .field("error_policy", &self.policy)
            .finish_non_exhaustive()
    }
}

/// Starts one process's state: a text measurer over a clone of the fonts (a
/// fresh lock, the font data shared) and a runner with the bundles registered
/// (names already checked).
fn start_local(options: &Options, fonts: &FontSet) -> Result<Local, Error> {
    let measurer = TextMeasurer::new(fonts.clone());
    let runner = backend::runner(options, &measurer)?;
    for b in &options.bundles {
        runner
            .register_bundle(&b.name, &b.source, &b.entry)
            .map_err(|e| Error::Bundle {
                name: b.name.clone(),
                message: match e {
                    surimono_islands::IslandError::MissingEntry { entry, .. } => format!(
                        "it doesn't define a global function {entry:?} (with a bundler, \
                         assign it to globalThis.{entry})"
                    ),
                    other => other.to_string(),
                },
            })?;
    }
    Ok(Local { measurer, runner })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;
    use std::sync::{Arc, PoisonError};
    use std::time::Duration;

    const T: &str = "application/vnd.surimono+json";

    /// A bring-your-own bundle for the tests: an empty SVG of the island's
    /// size, or, per its spec, a thrown error, a script over a limit, or text
    /// measured with the fonts.
    const TEST_BUNDLE: &str = r#"
        function entry(specJson, ctx) {
          const spec = JSON.parse(specJson);
          if (spec.throw) throw new Error(spec.throw);
          if (spec.limit === "time") { for (;;) {} }
          if (spec.limit === "memory") { const a = []; for (;;) a.push(new Array(1e5).fill(1)); }
          if (spec.limit === "stack") { const f = (n) => f(n + 1) + 1; f(0); }
          if (spec.raw) return spec.raw;
          const w = spec.measure ? ctx.measureText(spec.measure, "16px Noto Sans").width : 0;
          return `<svg xmlns="http://www.w3.org/2000/svg" width="${ctx.width}" height="${ctx.height}" data-w="${w.toFixed(2)}"></svg>`;
        }"#;

    fn test_font() -> Vec<u8> {
        let path = format!(
            "{}/../../fixtures/fonts/NotoSans-Regular.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn options() -> Options {
        Options::new()
            .font(test_font())
            .bundle("t", TEST_BUNDLE, "entry")
            .pool_size(2)
    }

    fn island(spec: &str) -> String {
        format!(
            "<script type=\"{T}\" data-renderer=\"t\" data-width=\"30\" data-height=\"20\">{spec}</script>"
        )
    }

    fn renderer(options: Options) -> Renderer {
        Renderer::new(options).unwrap_or_else(|e| panic!("{e}"))
    }

    const EMPTY: &str = "<div class=\"surimono-island\" data-renderer=\"t\"><svg xmlns=\"http://www.w3.org/2000/svg\" width=\"30\" height=\"20\" data-w=\"0.00\"></svg></div>";

    #[test]
    fn renderer_is_send_and_sync() {
        fn check<T: Send + Sync + 'static>() {}
        check::<Renderer>();
    }

    #[test]
    fn renders_islands_and_keeps_the_rest() {
        let r = renderer(options());
        let html = format!("<p>a</p>\n{}\n<p>b</p>", island("{}"));
        let out = r.render_islands(&html).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out, format!("<p>a</p>\n{EMPTY}\n<p>b</p>"));
        assert_eq!(
            r.render_islands(&html).ok(),
            Some(out),
            "same input, same output"
        );
    }

    #[test]
    fn script_failures_become_typed_errors() {
        let r = renderer(options().limits(Limits {
            time: Duration::from_millis(100),
            memory_bytes: 8 * 1024 * 1024,
            ..Limits::default()
        }));
        for (spec, kind) in [
            (r#"{"limit": "time"}"#, "time-limit"),
            (r#"{"limit": "memory"}"#, "memory-limit"),
            (r#"{"limit": "stack"}"#, "stack-overflow"),
            (r#"{"throw": "boom"}"#, "javascript"),
        ] {
            match r.render_islands(&island(spec)) {
                Err(e) => {
                    assert_eq!(e.kind(), kind, "{spec}: {e}");
                    assert_eq!(e.island().map(|i| i.index), Some(0));
                }
                Ok(out) => panic!("{spec}: expected {kind}, got {out}"),
            }
        }
        match r.render_islands(&island(r#"{"throw": "boom"}"#)) {
            Err(Error::JavaScript { message, .. }) => assert_eq!(message, "Error: boom"),
            other => panic!("{other:?}"),
        }
        // The renderer keeps working after every kind of failure.
        assert!(r.render_islands(&island("{}")).is_ok());
    }

    #[test]
    fn hostile_svg_cannot_break_out_of_the_wrapper() {
        // `<` is written \u003c in the spec, as in any marker, so the JSON can't
        // end the marker; the renderer returns it with real `<`s.
        let breakout = r#"{"raw": "\u003csvg>\u003c/svg>\u003c/div>\u003cscript>alert(1)\u003c/script>\u003ctextarea>"}"#;
        let r = renderer(options());
        match r.render_islands(&island(breakout)) {
            Err(Error::Render { island, message }) => {
                assert_eq!(island.index, 0);
                assert!(message.contains("SVG"), "{message}");
            }
            other => panic!("expected Error::Render, got {other:?}"),
        }
        let r = renderer(options().error_policy(ErrorPolicy::Placeholder));
        let out = r
            .render_islands(&format!("<main>{}<p>after</p></main>", island(breakout)))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            !out.contains("<script") && !out.contains("<textarea"),
            "{out}"
        );
        assert!(out.ends_with("--></div><p>after</p></main>"), "{out}");
        // Inside a well-formed SVG, scripts and handlers are removed.
        let inside = r#"{"raw": "\u003csvg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'>\u003cscript>alert(2)\u003c/script>\u003cforeignObject>\u003ctextarea xmlns='http://www.w3.org/1999/xhtml'>x\u003c/textarea>\u003c/foreignObject>\u003c/svg>"}"#;
        let out = r
            .render_islands(&island(inside))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!out.contains("script") && !out.contains("onload"), "{out}");
        assert!(
            out.starts_with("<div class=\"surimono-island\"") && out.ends_with("</svg></div>"),
            "{out}"
        );
    }

    #[test]
    fn text_is_measured_with_the_given_fonts() {
        let font = format!(
            "{}/../../fixtures/fonts/NotoSans-Regular.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        let Ok(bytes) = std::fs::read(&font) else {
            panic!("{font} is missing");
        };
        let r = renderer(options().font(bytes));
        let out = r
            .render_islands(&island(r#"{"measure": "Revenue"}"#))
            .unwrap_or_else(|e| panic!("{e}"));
        let w: f64 = out
            .split("data-w=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("{out}"));
        assert!(w > 40.0 && w < 80.0, "Revenue at 16px measured {w}");
        // Without fonts there is no renderer at all, never a system font.
        match Renderer::new(Options::new().bundle("t", TEST_BUNDLE, "entry")) {
            Err(Error::InvalidOptions(m)) => assert!(m.contains("no fonts"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_and_disabled_renderers() {
        let r = renderer(options());
        let html = format!("<script type=\"{T}\" data-renderer=\"plotly\">{{}}</script>");
        match r.render_islands(&html) {
            Err(Error::UnknownRenderer { island, enabled }) => {
                assert_eq!(island.renderer.as_deref(), Some("plotly"));
                assert_eq!(enabled.last().map(String::as_str), Some("t"));
            }
            other => panic!("expected UnknownRenderer, got {other:?}"),
        }
        let only = renderer(options().bundle("u", TEST_BUNDLE, "entry").renderers(["u"]));
        assert_eq!(only.renderers(), ["u".to_owned()]);
        assert!(matches!(
            only.render_islands(&island("{}")),
            Err(Error::UnknownRenderer { .. })
        ));
    }

    #[test]
    fn builtins_come_first_then_bundles_in_order() {
        let r = renderer(options().bundle("a", TEST_BUNDLE, "entry"));
        let mut want = backend::builtin_renderers();
        want.extend(["t".to_owned(), "a".to_owned()]);
        assert_eq!(r.renderers(), want.as_slice());
    }

    #[test]
    fn placeholder_policy() {
        let r = renderer(options().error_policy(ErrorPolicy::Placeholder));
        let out = r
            .render_islands(&format!("{}{}", island(r#"{"throw": "x"}"#), island("{}")))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            out.starts_with("<div class=\"surimono-island surimono-island-error\""),
            "{out}"
        );
        assert!(out.contains("data-error=\"javascript\""), "{out}");
        assert!(out.ends_with(EMPTY), "{out}");
    }

    #[test]
    fn invalid_options() {
        let bad_font = Renderer::new(options().font(b"nope".to_vec()));
        assert!(
            matches!(bad_font, Err(Error::Font { index: 1, .. })),
            "{bad_font:?}"
        );

        for limits in [
            Limits {
                memory_bytes: 10,
                ..Limits::default()
            },
            Limits {
                stack_bytes: 1,
                ..Limits::default()
            },
            Limits {
                time: Duration::ZERO,
                ..Limits::default()
            },
        ] {
            let r = Renderer::new(options().limits(limits));
            assert!(
                matches!(r, Err(Error::InvalidOptions(_))),
                "{limits:?}: {r:?}"
            );
        }
        for r in [
            Renderer::new(options().renderers(["nope"])),
            Renderer::new(options().pool_size(0)),
            Renderer::new(options().pool_size(65)),
            Renderer::new(options().utc_offset_minutes(15 * 60)),
            Renderer::new(options().now_ms(i64::MAX)),
        ] {
            assert!(matches!(r, Err(Error::InvalidOptions(_))), "{r:?}");
        }

        for (name, source, entry) in [
            ("", TEST_BUNDLE, "entry"),
            ("x", TEST_BUNDLE, "not a name"),
            ("t", TEST_BUNDLE, "entry"),
            ("x", "var nothing = 1;", "entry"),
            ("x", "throw new Error('broken bundle')", "entry"),
            ("x", "function (", "entry"),
        ] {
            let b = Renderer::new(options().bundle(name, source, entry));
            assert!(
                matches!(b, Err(Error::Bundle { .. })),
                "{name:?}/{source:?}: {b:?}"
            );
        }
        match Renderer::new(
            Options::new()
                .font(test_font())
                .bundle("x", "var y;", "draw"),
        ) {
            Err(Error::Bundle { message, .. }) => {
                assert!(message.contains("globalThis.draw"), "{message}");
                assert!(!message.contains("\"x\""), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn clock_options_reach_the_scripts() {
        let bundle = "function entry(s, ctx) { const d = new Date(); \
            return `<svg xmlns='http://www.w3.org/2000/svg' data-t='${Date.now()}' data-h='${d.getHours()}'/>`; }";
        let r = renderer(
            Options::new()
                .font(test_font())
                .bundle("clock", bundle, "entry")
                .now_ms(1_700_000_000_000)
                .utc_offset_minutes(540),
        );
        let out = r
            .render_islands(&format!(
                "<script type=\"{T}\" data-renderer=\"clock\">{{}}</script>"
            ))
            .unwrap_or_else(|e| panic!("{e}"));
        // 2023-11-14T22:13:20Z is 07:13 the next day in Japan.
        assert!(out.contains("data-t='1700000000000' data-h='7'"), "{out}");
    }

    #[test]
    fn many_threads_share_one_renderer() {
        let r = Arc::new(renderer(options()));
        let html = format!("<main>{}</main>", island("{}"));
        let expected = r.render_islands(&html).unwrap_or_else(|e| panic!("{e}"));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let r = Arc::clone(&r);
                let html = html.clone();
                std::thread::spawn(move || r.render_islands(&html))
            })
            .collect();
        for h in handles {
            match h.join() {
                Ok(Ok(out)) => assert_eq!(out, expected),
                other => panic!("{other:?}"),
            }
        }
    }

    /// What a forked child sees: the slot claimed last belongs to another pid.
    fn pretend_forked(r: &Renderer) {
        let last = last_claimed(&r.slots[..]);
        r.slots[last]
            .pid
            .store(u32::MAX - last as u32, Ordering::Release);
    }

    /// Tests that give up on another process's state push to the shared
    /// ABANDONED list; they run one at a time so counts stay exact.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn abandoned() -> usize {
        ABANDONED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    #[test]
    fn a_forked_child_starts_its_own_runner_and_measurer() {
        let _serial = serial();
        let r = renderer(options());
        let measured = island(r#"{"measure": "Revenue"}"#);
        let before = r
            .render_islands(&measured)
            .unwrap_or_else(|e| panic!("{e}"));
        pretend_forked(&r);
        assert_eq!(r.render_islands(&measured).ok(), Some(before.clone()));
        assert_eq!(last_claimed(&r.slots[..]), 1);
        assert_eq!(r.slots[1].pid.load(Ordering::Acquire), std::process::id());
        let (Some(parent), Some(child)) = (r.slots[0].state.get(), r.slots[1].state.get()) else {
            panic!("both processes' state should be started");
        };
        // A new measurer (its own lock), over the same font data.
        assert!(!std::ptr::eq(
            parent.measurer.fonts(),
            child.measurer.fonts()
        ));
        assert_eq!(
            parent.measurer.fonts().font_files().collect::<Vec<_>>(),
            child.measurer.fonts().font_files().collect::<Vec<_>>()
        );
        // A fork of the fork claims the next slot, and so on, with the same output.
        pretend_forked(&r);
        assert_eq!(r.render_islands(&measured).ok(), Some(before));
        assert_eq!(last_claimed(&r.slots[..]), 2);
    }

    #[test]
    fn many_threads_after_a_fork_share_one_new_slot() {
        let _serial = serial();
        let r = Arc::new(renderer(options()));
        let html = format!("<main>{}</main>", island(r#"{"measure": "Q3"}"#));
        let expected = r.render_islands(&html).unwrap_or_else(|e| panic!("{e}"));
        // Threads keep measuring in the "parent" state while the "child" starts.
        let stop = Arc::new(AtomicBool::new(false));
        let busy: Vec<_> = (0..2)
            .map(|_| {
                let (r, stop) = (Arc::clone(&r), Arc::clone(&stop));
                let parent = r.slots[0].state.get().map(|l| l.measurer.clone());
                std::thread::spawn(move || {
                    let Some(m) = parent else { return };
                    while !stop.load(Ordering::Relaxed) {
                        let _ = m.measure("busy", "12px Noto Sans");
                    }
                })
            })
            .collect();
        pretend_forked(&r);
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let r = Arc::clone(&r);
                let html = html.clone();
                std::thread::spawn(move || r.render_islands(&html))
            })
            .collect();
        for h in handles {
            match h.join() {
                Ok(Ok(out)) => assert_eq!(out, expected),
                other => panic!("{other:?}"),
            }
        }
        stop.store(true, Ordering::Relaxed);
        for b in busy {
            let _ = b.join();
        }
        assert_eq!(
            last_claimed(&r.slots[..]),
            1,
            "one slot for the new process"
        );
    }

    #[test]
    fn a_reused_pid_of_an_ancestor_is_not_this_process() {
        let _serial = serial();
        let r = renderer(options());
        // Slot 0 has our pid but slot 1 was claimed after it by another
        // process: slot 0 belonged to an ancestor whose pid came back.
        r.slots[1].pid.store(u32::MAX, Ordering::Release);
        assert!(r.render_islands(&island("{}")).is_ok());
        assert_eq!(last_claimed(&r.slots[..]), 2);
        // On drop, only slot 2 is this process's; slot 0 is abandoned.
        let before = abandoned();
        drop(r);
        assert_eq!(abandoned(), before + 1);
    }

    #[test]
    fn too_many_fork_generations_is_an_error_not_a_hang() {
        let _serial = serial();
        let r = renderer(options());
        for slot in r.slots.iter() {
            slot.pid.store(u32::MAX, Ordering::Release);
        }
        match r.render_islands(&island("{}")) {
            Err(Error::InvalidOptions(m)) => assert!(m.contains("fork generations"), "{m}"),
            other => panic!("{other:?}"),
        }
        // A document without islands still needs nothing.
        assert_eq!(
            r.render_islands("<p>x</p>").ok().as_deref(),
            Some("<p>x</p>")
        );
    }

    #[test]
    fn dropping_a_renderer_from_another_process_does_not_join_its_threads() {
        let _serial = serial();
        let r = renderer(options());
        assert!(r.render_islands(&island("{}")).is_ok());
        pretend_forked(&r);
        let before = abandoned();
        drop(r);
        assert_eq!(
            abandoned(),
            before + 1,
            "the parent's runner is kept, not joined"
        );
    }

    #[test]
    fn max_islands_caps_a_document() {
        let r = renderer(options().max_islands(2));
        assert!(r.render_islands(&island("{}").repeat(2)).is_ok());
        for policy in [ErrorPolicy::Fail, ErrorPolicy::Placeholder] {
            let r = renderer(options().max_islands(2).error_policy(policy));
            match r.render_islands(&island("{}").repeat(3)) {
                Err(e @ Error::TooManyIslands { max: 2 }) => {
                    assert_eq!(e.kind(), "too-many-islands");
                }
                other => panic!("{policy:?}: {other:?}"),
            }
        }
        assert!(matches!(
            Renderer::new(options().max_islands(0)),
            Err(Error::InvalidOptions(_))
        ));
        // The default is 1,000.
        let r = renderer(options());
        assert!(r.render_islands(&island("{}").repeat(1000)).is_ok());
        assert!(matches!(
            r.render_islands(&island("{}").repeat(1001)),
            Err(Error::TooManyIslands { max: 1000 })
        ));
    }

    #[test]
    fn a_cancelled_render_skips_the_remaining_islands() {
        let r = renderer(options());
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            r.render_islands_cancellable(&island("{}"), &cancel),
            Err(Error::Cancelled)
        ));
        // A document without islands has nothing to cancel.
        assert_eq!(
            r.render_islands_cancellable("<p>x</p>", &cancel)
                .ok()
                .as_deref(),
            Some("<p>x</p>")
        );
        cancel.store(false, Ordering::Release);
        assert_eq!(
            r.render_islands_cancellable(&island("{}"), &cancel).ok(),
            r.render_islands(&island("{}")).ok()
        );

        // Twenty islands that each run into a 100 ms time limit take 2 s;
        // cancelled after the first, the render ends at the next island.
        let slow = renderer(
            options()
                .error_policy(ErrorPolicy::Placeholder)
                .limits(Limits {
                    time: Duration::from_millis(100),
                    ..Limits::default()
                }),
        );
        let html = island(r#"{"limit": "time"}"#).repeat(20);
        let cancel = Arc::new(AtomicBool::new(false));
        let setter = {
            let cancel = Arc::clone(&cancel);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                cancel.store(true, Ordering::Release);
            })
        };
        let started = std::time::Instant::now();
        let out = slow.render_islands_cancellable(&html, &cancel);
        let took = started.elapsed();
        let _ = setter.join();
        assert!(matches!(out, Err(Error::Cancelled)), "{out:?}");
        assert!(took < Duration::from_millis(1500), "took {took:?}");
    }
}
