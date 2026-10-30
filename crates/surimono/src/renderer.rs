//! The [`Renderer`]: options in, islands rendered.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use surimono_gfx::FontSet;
use surimono_islands::{IslandRequest, IslandRunner};

use crate::Options;
use crate::backend;
use crate::error::Error;
use crate::prepass::{self, ErrorPolicy};
use crate::svg::SanitizedSvg;

/// Renders a document's islands to SVG.
///
/// Build one per configuration and reuse it: it keeps a pool of JavaScript
/// runtimes, compiles each renderer's bundle once, and measures text with the
/// fonts you gave it. It is `Send + Sync`, so one renderer can serve many
/// threads at a time.
///
/// ```
/// use surimono::{Options, Renderer};
///
/// let renderer = Renderer::new(Options::new())?;
/// let html = "<h1>Q3 report</h1><p>No charts on this page.</p>";
/// assert_eq!(renderer.render_islands(html)?, html);
/// # Ok::<(), surimono::Error>(())
/// ```
pub struct Renderer {
    /// The options it was built with (fonts moved out), to start the pool
    /// again in a forked child.
    options: Options,
    fonts: FontSet,
    pool: Mutex<Pool>,
    policy: ErrorPolicy,
    enabled: Vec<String>,
}

/// The runner and the process whose threads it runs on.
///
/// Worker threads don't survive `fork`: in a child (Puma or Unicorn workers
/// after `preload_app!`, Resque jobs) the runner would wait forever for
/// threads that don't exist. So the pool remembers the pid it was started in,
/// and the first render in another process starts a new one there.
struct Pool {
    pid: u32,
    /// Always `Some` until the renderer is dropped.
    runner: Option<Arc<IslandRunner>>,
}

/// Gives up on a runner whose threads belong to another process: dropping it
/// would join threads that don't exist here, which never returns. Its memory
/// is the parent's, copied on write; it is leaked once per fork.
fn abandon(runner: Arc<IslandRunner>) {
    #[allow(clippy::mem_forget, reason = "dropping it would hang; see above")]
    std::mem::forget(runner);
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let pool = self.pool.get_mut().unwrap_or_else(PoisonError::into_inner);
        if pool.pid != std::process::id() {
            if let Some(runner) = pool.runner.take() {
                abandon(runner);
            }
        }
    }
}

impl Renderer {
    /// Builds a renderer: loads the fonts, checks the limits, starts the
    /// runtime pool and compiles your bundles. Built-in renderers compile on
    /// first use.
    ///
    /// # Errors
    ///
    /// [`Error::Font`] for a font file that can't be read,
    /// [`Error::InvalidOptions`] for limits out of range (including a zero time
    /// limit), a pool size of 0 or an unknown name in [`Options::renderers`],
    /// and [`Error::Bundle`] for a bundle that can't be used.
    pub fn new(mut options: Options) -> Result<Self, Error> {
        if options.limits.time.is_zero() {
            return Err(Error::InvalidOptions(
                "limits.time is zero; every island would fail".into(),
            ));
        }
        if options.pool_size == Some(0) {
            return Err(Error::InvalidOptions("pool_size must be at least 1".into()));
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
        let runner = start_runner(&options, &fonts)?;
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
        Ok(Self {
            policy: options.error_policy,
            options,
            fonts,
            pool: Mutex::new(Pool {
                pid: std::process::id(),
                runner: Some(Arc::new(runner)),
            }),
            enabled,
        })
    }

    /// The runner for this process, started again if this is a forked child.
    fn runner(&self) -> Result<Arc<IslandRunner>, Error> {
        let pid = std::process::id();
        let mut pool = self.pool.lock().unwrap_or_else(PoisonError::into_inner);
        if pool.pid != pid || pool.runner.is_none() {
            let fresh = Arc::new(start_runner(&self.options, &self.fonts)?);
            if let Some(old) = pool.runner.replace(fresh) {
                abandon(old);
            }
            pool.pid = pid;
        }
        pool.runner
            .clone()
            .ok_or_else(|| Error::InvalidOptions("the renderer has no runner".into()))
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
    /// write `<` as `\u003c` (Rails' `to_json` does), so no `</script>` or
    /// `<!--` in a string can end or confuse the marker.
    ///
    /// Every renderer's SVG is sanitised before it goes in: no scripts, event
    /// handlers, external references or markup that could end the wrapper; ids
    /// get a per-island prefix; font families map to your fonts. SVG that can't
    /// be made safe fails the island with [`Error::Render`].
    ///
    /// Islands render in document order. The same renderer, options and input
    /// always give the same bytes.
    ///
    /// # Errors
    ///
    /// With [`ErrorPolicy::Fail`] (the default), the first island that fails ends
    /// the render with its error: [`Error::InvalidIsland`],
    /// [`Error::UnknownRenderer`], [`Error::MemoryLimit`], [`Error::TimeLimit`],
    /// [`Error::StackOverflow`], [`Error::JavaScript`] or [`Error::Render`]. With
    /// [`ErrorPolicy::Placeholder`] failed islands become placeholders instead.
    /// [`Error::Html`] if the markup is too ambiguous to rewrite safely.
    pub fn render_islands(&self, html: &str) -> Result<String, Error> {
        // Documents without islands never need the runner.
        let mut runner: Option<Arc<IslandRunner>> = None;
        prepass::rewrite_islands(html, self.policy, |island| {
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
            if runner.is_none() {
                runner = Some(self.runner()?);
            }
            let svg = runner
                .as_ref()
                .ok_or_else(|| Error::InvalidOptions("the renderer has no runner".into()))?
                .render(&request)
                .map_err(|e| backend::island_error(e, island.at.clone(), &self.enabled))?;
            SanitizedSvg::new(&svg, &island.at, &self.fonts)
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

/// Starts a runner and registers the bundles (names already checked).
fn start_runner(options: &Options, fonts: &FontSet) -> Result<IslandRunner, Error> {
    let runner = backend::runner(options, fonts)?;
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
    Ok(runner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;
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

    fn options() -> Options {
        Options::new()
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
        // Without fonts, measuring is a JavaScript error, never a system font.
        let none = renderer(options());
        assert!(matches!(
            none.render_islands(&island(r#"{"measure": "x"}"#)),
            Err(Error::JavaScript { .. })
        ));
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
            matches!(bad_font, Err(Error::Font { index: 0, .. })),
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
        match Renderer::new(Options::new().bundle("x", "var y;", "draw")) {
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

    #[test]
    fn a_runner_from_another_process_is_replaced_not_joined() {
        let r = renderer(options());
        let before = r
            .render_islands(&island("{}"))
            .unwrap_or_else(|e| panic!("{e}"));
        // What a forked child sees: a pool started under another pid.
        r.pool.lock().unwrap_or_else(PoisonError::into_inner).pid = u32::MAX;
        assert_eq!(r.render_islands(&island("{}")).ok(), Some(before));
        assert_eq!(
            r.pool.lock().unwrap_or_else(PoisonError::into_inner).pid,
            std::process::id()
        );
        // Dropping a renderer whose pool belongs to another process doesn't
        // join (and hang on) that pool's threads.
        r.pool.lock().unwrap_or_else(PoisonError::into_inner).pid = u32::MAX;
        drop(r);
    }
}
