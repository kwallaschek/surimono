//! The [`Renderer`]: options in, islands rendered.

use std::fmt;

use crate::Options;
use crate::backend::{self, FontSet, IslandRequest, IslandRunner, RunnerOptions, TextMeasurer};
use crate::error::Error;
use crate::prepass::{self, ErrorPolicy};

/// Renders a document's islands to SVG.
///
/// Build one per configuration and reuse it: it keeps a pool of JavaScript
/// runtimes with the renderers' bundles compiled once. It is `Send + Sync`, so
/// one renderer can serve many threads at a time.
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
    runner: IslandRunner,
    policy: ErrorPolicy,
    enabled: Vec<String>,
}

impl Renderer {
    /// Builds a renderer: loads the fonts, checks the limits, sets up the
    /// runtime pool and compiles the bundles.
    ///
    /// # Errors
    ///
    /// [`Error::Font`] for a font file that can't be read,
    /// [`Error::InvalidOptions`] for limits out of range or an unknown name in
    /// [`Options::renderers`], and [`Error::Bundle`] for a bundle that can't be used.
    pub fn new(options: Options) -> Result<Self, Error> {
        let mut fonts = FontSet::new();
        for (index, bytes) in options.fonts.into_iter().enumerate() {
            fonts
                .add_font_bytes(bytes)
                .map_err(|reason| Error::Font { index, reason })?;
        }
        let mut runner = IslandRunner::new(RunnerOptions {
            limits: options.limits,
            pool_size: options.pool_size.unwrap_or_else(backend::default_pool_size),
            seed: options.seed,
            measurer: TextMeasurer::new(fonts),
        })
        .map_err(|e| Error::InvalidOptions(e.to_string()))?;
        for b in &options.bundles {
            runner
                .register_bundle(&b.name, &b.source, &b.entry)
                .map_err(|e| Error::Bundle {
                    name: b.name.clone(),
                    message: e.to_string(),
                })?;
        }
        let available = runner.renderer_names();
        let enabled = match options.renderers {
            None => available,
            Some(wanted) => {
                if let Some(unknown) = wanted.iter().find(|w| !available.contains(w)) {
                    return Err(Error::InvalidOptions(format!(
                        "renderer {unknown:?} doesn't exist; available: {}",
                        available.join(", ")
                    )));
                }
                let mut enabled = Vec::with_capacity(wanted.len());
                for w in wanted {
                    if !enabled.contains(&w) {
                        enabled.push(w);
                    }
                }
                enabled
            }
        };
        Ok(Self {
            runner,
            policy: options.error_policy,
            enabled,
        })
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
    /// `div` is allowed, not inside `<p>`, `<span>` or `<a>`. Write `</script>`
    /// inside a JSON string as `<\/script>`.
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
            self.runner
                .render(&request)
                .map_err(|e| backend::island_error(e, island.at.clone(), &self.enabled))
        })
    }

    /// The renderers islands may name, in the order they were enabled.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;
    use crate::backend::STUB_RENDERER;
    use std::time::Duration;

    const T: &str = "application/vnd.surimono+json";

    fn island(spec: &str) -> String {
        format!(
            "<script type=\"{T}\" data-renderer=\"{STUB_RENDERER}\" data-width=\"30\" data-height=\"20\">{spec}</script>"
        )
    }

    fn renderer(options: Options) -> Renderer {
        Renderer::new(options).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn renderer_is_send_and_sync() {
        fn check<T: Send + Sync + 'static>() {}
        check::<Renderer>();
    }

    #[test]
    fn renders_islands_and_keeps_the_rest() {
        let r = renderer(Options::new());
        let html = format!("<p>a</p>\n{}\n<p>b</p>", island("{}"));
        assert_eq!(
            r.render_islands(&html).unwrap_or_else(|e| panic!("{e}")),
            "<p>a</p>\n<div class=\"surimono-island\" data-renderer=\"surimono-stub\">\
             <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"30\" height=\"20\" \
             viewBox=\"0 0 30 20\"></svg></div>\n<p>b</p>"
        );
    }

    #[test]
    fn script_failures_become_typed_errors() {
        let r = renderer(Options::new().limits(Limits {
            time: Duration::from_millis(50),
            ..Limits::default()
        }));
        match r.render_islands(&island("{\"busy_ms\": 5000}")) {
            Err(Error::TimeLimit { island }) => assert_eq!(island.index, 0),
            other => panic!("expected TimeLimit, got {other:?}"),
        }
        assert!(matches!(
            r.render_islands(&island("{\"limit\": \"memory\"}")),
            Err(Error::MemoryLimit { .. })
        ));
        assert!(matches!(
            r.render_islands(&island("{\"limit\": \"stack\"}")),
            Err(Error::StackOverflow { .. })
        ));
        match r.render_islands(&island("{\"throw\": \"boom\"}")) {
            Err(Error::JavaScript { message, stack, .. }) => {
                assert_eq!(message, "Error: boom");
                assert!(stack.is_some());
            }
            other => panic!("expected JavaScript, got {other:?}"),
        }
    }

    #[test]
    fn unknown_and_disabled_renderers() {
        let r = renderer(Options::new());
        let html = format!("<script type=\"{T}\" data-renderer=\"plotly\">{{}}</script>");
        match r.render_islands(&html) {
            Err(Error::UnknownRenderer { island, enabled }) => {
                assert_eq!(island.renderer.as_deref(), Some("plotly"));
                assert_eq!(enabled, vec![STUB_RENDERER.to_owned()]);
            }
            other => panic!("expected UnknownRenderer, got {other:?}"),
        }
        let none = renderer(
            Options::new()
                .bundle("mine", "function f(){}", "f")
                .renderers(["mine"]),
        );
        assert_eq!(none.renderers(), ["mine".to_owned()]);
        assert!(matches!(
            none.render_islands(&island("{}")),
            Err(Error::UnknownRenderer { .. })
        ));
    }

    #[test]
    fn placeholder_policy() {
        let r = renderer(Options::new().error_policy(ErrorPolicy::Placeholder));
        let out = r
            .render_islands(&format!("{}{}", island("{\"throw\": \"x\"}"), island("{}")))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            out.starts_with("<div class=\"surimono-island surimono-island-error\""),
            "{out}"
        );
        assert!(out.contains("data-error=\"javascript\""), "{out}");
        assert!(out.ends_with("viewBox=\"0 0 30 20\"></svg></div>"), "{out}");
    }

    #[test]
    fn invalid_options() {
        let bad_font = Renderer::new(Options::new().font(b"OTTO".to_vec()).font(b"nope".to_vec()));
        assert!(
            matches!(bad_font, Err(Error::Font { index: 1, .. })),
            "{bad_font:?}"
        );

        let tiny = Renderer::new(Options::new().limits(Limits {
            memory_bytes: 10,
            ..Limits::default()
        }));
        assert!(matches!(tiny, Err(Error::InvalidOptions(_))), "{tiny:?}");

        let unknown = Renderer::new(Options::new().renderers(["nope"]));
        match unknown {
            Err(Error::InvalidOptions(m)) => assert!(m.contains("\"nope\""), "{m}"),
            other => panic!("{other:?}"),
        }

        let pool = Renderer::new(Options::new().pool_size(0));
        assert!(matches!(pool, Err(Error::InvalidOptions(_))), "{pool:?}");

        for (name, entry) in [("", "f"), ("x", "not a name"), (STUB_RENDERER, "f")] {
            let b = Renderer::new(Options::new().bundle(name, "function f(){}", entry));
            assert!(
                matches!(b, Err(Error::Bundle { .. })),
                "{name:?}/{entry:?}: {b:?}"
            );
        }
    }

    #[test]
    fn many_threads_share_one_renderer() {
        let r = std::sync::Arc::new(renderer(Options::new()));
        let html = format!("<main>{}</main>", island("{\"busy_ms\": 5}"));
        let expected = r.render_islands(&html).unwrap_or_else(|e| panic!("{e}"));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let r = std::sync::Arc::clone(&r);
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
}
