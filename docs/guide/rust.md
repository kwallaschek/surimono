# Rust

The `surimono` crate is the whole public API: a `Renderer` built from `Options`,
one method that renders a document's islands, and one error type. This guide
pairs it with [fulgur](https://github.com/fulgur-rs/fulgur) to print PDFs.

For the marker format and the renderers, see the [islands guide](islands.md).

## Add the crates

```toml
[dependencies]
surimono = "0.1"
fulgur = "0.40"   # surimono 0.1 is tested with fulgur 0.40.0
```

surimono needs Rust 1.87 or later. It builds QuickJS from C sources, so a C
compiler must be installed (as for most `-sys` crates).

<!-- confirm: facade feature names once P1-PKG-02 wires in the renderers -->
ECharts and Vega are Cargo features, `echarts` and `vega`, both on by default.
Each adds its library's JavaScript to your binary. Leave out the one you don't
use:

```toml
surimono = { version = "0.1", default-features = false, features = ["echarts"] }
```

## A pipeline

Build the renderer and the fulgur engine once, with the same font bytes, and
reuse them for every document:

```rust
use surimono::{Options, Renderer};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub struct PdfPipeline {
    renderer: Renderer,
    engine: fulgur::Engine,
}

impl PdfPipeline {
    pub fn new(font_paths: &[&str]) -> Result<Self, BoxError> {
        let mut options = Options::new();
        let mut assets = fulgur::AssetBundle::new();
        for path in font_paths {
            let bytes = std::fs::read(path)?;
            options = options.font(bytes.clone()); // chart text
            assets.add_font_bytes(bytes)?;         // page text
        }
        Ok(Self {
            renderer: Renderer::new(options)?,
            engine: fulgur::Engine::builder().assets(assets).system_fonts(false).build(),
        })
    }

    /// HTML with island markers in, HTML with inline SVG out.
    pub fn islands(&self, html: &str) -> Result<String, surimono::Error> {
        self.renderer.render_islands(html)
    }

    /// HTML with island markers in, PDF bytes out.
    pub fn render(&self, html: &str) -> Result<Vec<u8>, BoxError> {
        Ok(self.engine.render(&self.islands(html)?)?)
    }
}
```

- `Renderer::new` checks everything up front: fonts that don't load, limits out
  of range, unknown renderer names and bundles that don't compile fail here, at
  start-up, not on the first document.
- `Renderer` is `Send + Sync`. Put the pipeline in an `Arc` and share it
  between threads. It keeps a pool of JavaScript runtimes, sized with
  `Options::pool_size` (default: about the number of CPUs).
- `render_islands` blocks the calling thread until the islands are drawn. In
  async code, call it from `spawn_blocking`:

  ```rust
  let pipeline = Arc::clone(&state.pipeline);
  let pdf = tokio::task::spawn_blocking(move || pipeline.render(&html)).await??;
  ```

- Don't build a renderer and then `fork` the process. Its runtimes live on
  threads, which don't survive a fork. Build it in the process that uses it.

## Fonts

Pass the same font files, in the same order, to `Options::font` and to fulgur's
`AssetBundle`. Only TrueType, OpenType and collection files are accepted; WOFF
and WOFF2 give `Error::Font`. Order matters: characters a font lacks fall back
to the next font in the order you added them.

**Chart text is drawn as outlines.** By default surimono turns every label in an
island into vector paths, from your fonts, with the same shaping it measured
with. fulgur 0.40 prints `<svg>` text with the machine's system fonts, not the
fonts in its `AssetBundle`, so real text would print in a different font from
the one it was measured with. Outlines print exactly as measured. The cost is
that chart text can't be selected, searched or copied in the PDF. The
[islands guide](islands.md#chart-text-is-drawn-as-outlines) has the details.

To keep real text, set `TextMode::Keep` in the options. Only do this with an
engine that prints SVG text with your registered fonts:

<!-- confirm: builder method name for TextMode once the gfx/facade change lands -->
```rust
use surimono::TextMode;

let options = options.text_mode(TextMode::Keep);
```

**Name registered families in page CSS.** surimono leaves the rest of the page
alone. With `system_fonts(false)`, which keeps fulgur's output the same on every
machine, fulgur finds no font for a generic family such as `sans-serif`, and
text set in it alone prints blank. Write `font-family: "Noto Sans", "Noto Sans
JP"` using the family names stored in your font files.

## Options

```rust
use std::time::Duration;
use surimono::{ErrorPolicy, Limits, Options};

let options = Options::new()
    .font(std::fs::read("fonts/NotoSans-Regular.ttf")?)
    .font(std::fs::read("fonts/NotoSansJP-Regular.otf")?)
    .limits(Limits { time: Duration::from_millis(500), ..Limits::default() })
    .seed(7)
    .error_policy(ErrorPolicy::Placeholder)
    .renderers(["echarts", "sparkline"])
    .bundle("sparkline", std::fs::read_to_string("pdf/sparkline.js")?, "render")
    .pool_size(4);
```

| Method | Default | What it does |
|---|---|---|
| `font(bytes)` | none | Adds a font file for measuring and drawing chart text. Call once per file. |
| `limits(Limits)` | 64 MiB, 2 s, 512 KiB | Heap, time and stack limits for each island's script |
| `seed(u64)` | 0 | Seed for `Math.random` |
| `error_policy(ErrorPolicy)` | `Fail` | `Fail` stops at the first failed island; `Placeholder` leaves an empty element and continues |
| `renderers(names)` | all | Enables only these renderers. Islands naming any other fail with `Error::UnknownRenderer`. |
| `bundle(name, source, entry)` | none | Adds your own renderer; see [the islands guide](islands.md#your-own-renderer) |
| `pool_size(n)` | about the CPU count | How many islands can render at once |

`Limits` has public fields (`memory_bytes`, `time`, `stack_bytes`) and the
constants `Limits::DEFAULT_MEMORY_BYTES`, `DEFAULT_TIME` and
`DEFAULT_STACK_BYTES`. The memory limit must be at least 1 MiB and the stack
limit between 64 KiB and 16 MiB. The time limit applies to each island, not to
the whole document.

## Errors

`surimono::Error` is one `#[non_exhaustive]` enum, so match it with a wildcard
arm:

```rust
use surimono::Error;

fn islands_or_log(pipeline: &PdfPipeline, html: &str) -> Option<String> {
    match pipeline.islands(html) {
        Ok(html) => Some(html),
        Err(Error::JavaScript { island, message, stack, .. }) => {
            eprintln!("{island}: {message}\n{}", stack.unwrap_or_default());
            None
        }
        Err(e) => {
            eprintln!("{e} [{}]", e.kind());
            None
        }
    }
}
```

Most errors are deterministic: the same document fails the same way every
time, so retrying won't help. `Error::TimeLimit` is the exception, since how
far a script gets in its time depends on how busy the machine is.

- Island errors (`InvalidIsland`, `UnknownRenderer`, `MemoryLimit`, `TimeLimit`,
  `StackOverflow`, `JavaScript`, `Render`) carry an `IslandRef` with `index`
  (from 0), the marker's `id` and its `renderer`. `Error::island()` returns it
  for any error that has one.
- `Font`, `Bundle` and `InvalidOptions` come from `Renderer::new`. `Html` means
  the markup was too ambiguous to rewrite safely.
- `Error::kind()` returns a stable kebab-case name (`"time-limit"`), the same
  one the Ruby gem and placeholders use.

## Same input, same output

With the same options and HTML, `render_islands` returns the same bytes, on any
machine: `Math.random` is seeded, `Date` is fixed, and only your fonts are used.
That makes whole-document assertions practical:

```rust
#[test]
fn report_renders_the_same_twice() {
    let pipeline = PdfPipeline::new(&["fonts/NotoSans-Regular.ttf"]).unwrap();
    let html = std::fs::read_to_string("templates/report.html").unwrap();
    let first = pipeline.islands(&html).unwrap();
    assert!(first.contains(r#"<div class="surimono-island""#));
    assert_eq!(first, pipeline.islands(&html).unwrap());
}
```

A runnable version of the island step, with its tests, is in
[`examples/rust-islands`](../../examples/rust-islands).
