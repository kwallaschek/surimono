# Rust

The `surimono` crate is the whole public API: a `Renderer` built from `Options`,
a method that renders a document's islands (and a cancellable variant), and one
error type. This guide
pairs it with [fulgur](https://github.com/fulgur-rs/fulgur) to print PDFs.

For the marker format and the renderers, see the [islands guide](islands.md).

## Add the crates

surimono's crates aren't on crates.io yet. surimono needs a patched QuickJS,
and a crates.io release can't carry the patch until the fixes are upstream or
published under crate names of our own. Until then, depend on the repository
and patch `rquickjs-sys` in your own `Cargo.toml`, because a dependency's
`[patch]` never reaches your build:

```toml
[dependencies]
surimono = { git = "https://github.com/kwallaschek/surimono", branch = "main" }
fulgur = "0.40"   # surimono is tested with fulgur 0.40.0

[patch.crates-io]
rquickjs-sys = { git = "https://github.com/kwallaschek/surimono", branch = "main" }
```

Without the `[patch.crates-io]` entry the build fails at link time with
`undefined reference to surimono_quickjs_patch_level`. Cargo finds
`rquickjs-sys` in the repository's `vendor/` folder. Keep both entries on the
same branch or `rev` so they come from one commit.

surimono needs Rust 1.87 or later. fulgur 0.40 depends on krilla 0.7, which
needs Rust 1.89, so the pair needs 1.89. surimono builds QuickJS from C
sources, so a C compiler must be installed (as for most `-sys` crates). On a
toolchain older than 1.88, put `edition = "2024"` and a `rust-version` in your
package so that Cargo picks dependency versions your toolchain can build; some
current releases need 1.88.

ECharts and Vega are Cargo features, both on by default: `echarts` adds the
`echarts` renderer, `vega` adds `vega` and `vega-lite`. Each adds its library,
as precompiled QuickJS bytecode, to your binary. Leave out the one you don't
use:

```toml
surimono = { git = "https://github.com/kwallaschek/surimono", branch = "main", default-features = false, features = ["echarts"] }
```

`surimono::builtin_renderers()` lists the built-ins in your build, and
`surimono::precompiled_builtins()` says how many of them loaded precompiled
bytecode. The two counts match in a normal build. If the second is lower
(`SURIMONO_PRECOMPILE=0` was set when building or at start-up, or a cross
build produced bytecode that doesn't fit the engine), output is the same, but
each built-in's first render in a process compiles its library from source and
is slower. A test that the counts match catches a broken build early.

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
  between threads. It keeps a pool of JavaScript runtimes that starts with one
  and grows only when renders overlap, up to `Options::pool_size` (default: the
  number of CPUs, at most 4; at most 64). Idle runtimes free their memory.
- `render_islands` blocks the calling thread until the islands are drawn. In
  async code, call it from `spawn_blocking`:

  ```rust
  let pipeline = Arc::clone(&state.pipeline);
  let pdf = tokio::task::spawn_blocking(move || pipeline.render(&html)).await??;
  ```

- A renderer survives `fork`. Its runtimes live on threads, which don't, so
  the first render in a forked child starts the child's own pool and text
  measurer, sharing the font data with the parent and taking no lock a parent
  thread could have held. fulgur makes no such promise, so build its engine
  in the child, after the fork.

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

Each label becomes a `<g role="img" aria-label="…">`, so the words stay in the
SVG for accessibility. To keep real text, set `TextMode::Keep` in the options.
Only do this with an engine that prints SVG text with your registered fonts:

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
    .pool_size(4)
    .max_islands(50)
    .now_ms(1_775_001_600_000) // 2026-04-01T00:00:00Z
    .utc_offset_minutes(540); // Japan
```

| Method | Default | What it does |
|---|---|---|
| `font(bytes)` | none | Adds a font file for measuring and drawing chart text. Call once per file. |
| `limits(Limits)` | 64 MiB, 2 s, 512 KiB | Heap, time and stack limits for each island's script |
| `seed(u64)` | 0 | Seed for `Math.random` |
| `error_policy(ErrorPolicy)` | `Fail` | `Fail` stops at the first failed island; `Placeholder` leaves an empty element and continues |
| `renderers(names)` | all | Enables only these renderers. Islands naming any other fail with `Error::UnknownRenderer`. |
| `bundle(name, source, entry)` | none | Adds your own renderer; see [the islands guide](islands.md#your-own-renderer) |
| `pool_size(n)` | CPUs, at most 4 | Most islands rendered at once (1 to 64) |
| `max_islands(n)` | 1,000 | Most island markers one document may have; more fails with `Error::TooManyIslands` before any island runs |
| `now_ms(i64)` | `DEFAULT_NOW_MS` (2026-01-01T00:00:00Z) | What `Date.now()` and `new Date()` return, in ms since the epoch |
| `utc_offset_minutes(i32)` | 0 | Local time's offset from UTC for scripts (-720 to 840) |
| `text_mode(TextMode)` | `Outlines` | `Outlines` draws chart text as paths in your fonts; `Keep` leaves `<text>` |

The host's clock and time zone are never read.

`Limits` has public fields (`memory_bytes`, `time`, `stack_bytes`) and the
constants `Limits::DEFAULT_MEMORY_BYTES`, `DEFAULT_TIME` and
`DEFAULT_STACK_BYTES`. The memory limit must be at least 1 MiB and the stack
limit between 64 KiB and 16 MiB. The time limit covers loading the renderer
and drawing one island, not the whole document, so a document with N islands
can take up to N × `time`. `max_islands` caps N.

## Cancelling a render

`render_islands_cancellable` takes an `AtomicBool` and checks it before each
island; once it is true, the rest are skipped with `Error::Cancelled`, whatever
the error policy. An island already running ends at its own time limit. Set the
flag from another thread, a timer or a signal handler:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

let cancel = Arc::new(AtomicBool::new(false));
let flag = Arc::clone(&cancel);
std::thread::spawn(move || {
    std::thread::sleep(Duration::from_secs(10));
    flag.store(true, Ordering::Relaxed);
});
let result = renderer.render_islands_cancellable(&html, &cancel);
```

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
  the markup was too ambiguous to rewrite safely. `TooManyIslands { max }` and
  `Cancelled` belong to the whole document and end it whatever the policy.
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
