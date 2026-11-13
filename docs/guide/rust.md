# Rust

The `surimono` crate is the whole public API: a `Renderer` built from `Options`,
methods that render a document's islands or run it as a page (each with a
cancellable variant), and one error type. This guide pairs it with
[fulgur](https://github.com/fulgur-rs/fulgur) to print PDFs.

For the marker format and the renderers, see the [islands guide](islands.md).

## Add the crates

surimono isn't released yet; the first release is planned for v0.3. Its crates
aren't on crates.io. surimono needs a patched QuickJS, and a crates.io release
can't carry the patch until the fixes are upstream or published under crate
names of our own. Until then, depend on the repository and patch `rquickjs-sys`
in your own `Cargo.toml`, because a dependency's `[patch]` never reaches your
build:

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

surimono needs Rust 1.89 or later, as does fulgur 0.40 (through krilla 0.7).
surimono builds QuickJS from C sources, so a C compiler must be installed (as
for most `-sys` crates).

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
- A `fork` holds new pages back and waits for running ones, for up to 5 seconds,
  so a child doesn't inherit a lock one of them held. A page can run for three
  times the time limit (parse, scripts, the rest). If it outlasts the wait, the
  `fork` goes ahead and the child may hang at such a lock, so keep the limit
  short in a process that forks. `posix_spawn` doesn't wait.

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
| `layout_queries(LayoutQueries)` | `Zeros` | What a page's layout queries give; see [Layout queries](#layout-queries) |
| `keep_scripts(bool)` | `false` | Leaves `<script>` elements in a page's output |
| `printer_safe(bool)` | `true` | Writes a page's output in forms fulgur prints the way Chrome prints the page; see [Printer-safe output](#printer-safe-output) |
| `resource_loader(ResourceLoader)` | none | What a page may load; see [External files and frames](#external-files-and-frames) |

The host's clock and time zone are never read.

### Limits

Build `Limits` from the defaults, so a limit added later keeps its default in
your code. The constants are `Limits::DEFAULT_MEMORY_BYTES`, `DEFAULT_TIME`,
`DEFAULT_STACK_BYTES` and `DEFAULT_OUTPUT_BYTES`.

| Field | Default | Allowed | For each island | For a page |
|---|---|---|---|---|
| `memory_bytes` | 64 MiB | at least 1 MiB | A fresh JavaScript heap | One heap for all its scripts |
| `time` | 2 s | more than zero | One deadline over loading the renderer and drawing | One deadline for all its scripts, timers included; another of the same length for its parse, and another for everything after the scripts |
| `stack_bytes` | 512 KiB | 64 KiB to 16 MiB | The engine's native stack | One limit for all its scripts |
| `output_bytes` | 64 MiB | at least 1 MiB | Not used | The most HTML `render_page` gives back, and the most text a page may hold while it runs |

Values out of range fail in `Renderer::new` with `Error::InvalidOptions`. A
document with N islands can take up to N × `time`. `max_islands` caps N. A page
takes at most about 3 × `time` before its islands, each of which then has its
own `time`.

## Cancelling a render

`render_islands_cancellable` takes an `AtomicBool` and reads it about every
5 ms while an island runs. Once it is true, the running island's script stops
within milliseconds, the islands not yet started are skipped, and the render
ends with `Error::Cancelled`, whatever the error policy. Leave the flag set
until the call returns. Set it from another thread, a timer or a signal handler:

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

Loading a bundle isn't interrupted: a cancel takes effect when the load ends.
Precompiled built-ins load in place. A built-in compiled from source on first
use, or your own bundle compiled at a forked process's first render, can take
up to twice `time` (one limit to compile, one to run it once), plus any wait
for a free runtime.

## Page scripts

Island markers suit charts you can describe as JSON. When the document's own
`<script>` elements build it, `render_page` runs them as a browser would, on a
virtual clock until the page settles, then draws any island markers.
`render_islands` never runs a page's scripts.

```rust
use surimono::{Error, Options, Renderer, SettleReason};

fn main() -> Result<(), Error> {
    let font = std::fs::read("fixtures/fonts/NotoSans-Regular.ttf").expect("the font");
    let renderer = Renderer::new(Options::new().font(font))?;

    let page = renderer.render_page(
        "<!doctype html><p id=a>?</p>
         <script>setTimeout(() => { document.querySelector('p').textContent = 'done'; }, 1000);</script>",
    )?;

    assert!(page.html.contains("<p id=\"a\">done</p>"));
    assert_eq!(page.report.settled, SettleReason::Idle);
    Ok(())
}
```

`page.html` goes to fulgur. To print in the pipeline above:

```rust
impl PdfPipeline {
    /// A page with scripts in, PDF bytes out. Logs what went wrong in the page.
    pub fn render_page(&self, html: &str) -> Result<Vec<u8>, BoxError> {
        let page = self.renderer.render_page(html)?;
        for error in &page.report.errors {
            eprintln!("page: {error}");
        }
        Ok(self.engine.render(&page.html)?)
    }
}
```

A `Renderer` starts the threads for pages on the first page, up to
`Options::pool_size`. Like `render_islands`, the call blocks; use
`spawn_blocking` in async code.

### Report or error

A page's own mistakes go in `PageReport` and the page goes on, as in a browser.
A limit is an `Err`, and there is no page.

| `PageReport` field | What it says |
|---|---|
| `settled` | `SettleReason::Idle` (nothing left to run), `VirtualBudget` (timers left after 10 seconds of virtual time, as with a `setInterval` nobody clears) or `TaskLimit` (100,000 tasks, with more due) |
| `uncaught`, `errors` | Exceptions nothing caught, and the first few as messages |
| `unhandled_rejections`, `rejections` | Promises rejected with no handler, and the first few reasons |
| `failed_count`, `failed` | External scripts, images and stylesheets the page didn't get, and the first few as `ResourceFailure { kind, url, reason }` |

`PageReport`, `PageOutput`, `ResourceFailure`, `SettleReason` and the kinds and
reasons are `#[non_exhaustive]`. A report isn't always clean on a page that
printed well: `VirtualBudget` is normal for a page with a clock in it, and every
`<img src>` is listed, because surimono leaves images to the printer.

| Error | When |
|---|---|
| `Error::PageMemoryLimit` | The page's scripts used more than `memory_bytes` |
| `Error::PageTimeLimit` | The page ran past `time`; see [Limits](#limits) |
| `Error::PageStackOverflow` | The page's scripts recursed deeper than `stack_bytes` allows, and nothing caught it |
| `Error::PageDomLimit { message }` | The page or its scripts passed a limit on the DOM (nodes, depth, text, attributes), or its CSS passed a limit, or its output is over `output_bytes`. The message says which. |
| `Error::PageLayoutQuery { member }` | Under `LayoutQueries::Throw`, a script asked for layout through `member` |
| `Error::PageFailed { message }` | The page couldn't run, or used something the DOM defines and surimono doesn't do yet |

Whatever the `ErrorPolicy`, these end the render. Islands in a page fail as in
`render_islands`, under the policy. Their `kind()` strings are those of the
island errors for the three script limits, then `dom-limit`, `layout-query` and
`page`.

### What comes out

`page.html` is the document serialized from a parse, not your input with changes,
so it isn't byte-identical to what you passed in:

- Attributes are quoted, a missing `<head>` or `<body>` is added, and comments
  are gone.
- `<noscript>` is gone, and `<script>` too, except island markers on their way
  to becoming SVG. `Options::keep_scripts(true)` keeps the scripts, for an engine
  that would run them.
- A page with no doctype or an old one comes out with no doctype;
  a standards-mode page comes out with `<!doctype html>`.
- The output is screened so the printer's parse matches ours.
- The output is printer-safe, unless `Options::printer_safe(false)` says
  otherwise. See below.

#### Printer-safe output

fulgur 0.40 prints some ordinary pages wrong although their DOM is right. For
three of those gaps there is a form of the page that it prints right, and by
default `page.html` is written in that form:

- A block `<svg>` keeps the size its `width` and `height` give it, and a border
  on one side prints in its own colour, not the text's. Both come from one
  `<style>` added in `<head>`, before the page's own style sheets, whose rules
  rank below all of the page's CSS.
- A `<pre>` whose text ends in a newline loses that newline, which a browser
  doesn't print, so the box isn't a line taller.

Each is made only where Chrome prints the result exactly as it prints the page,
whatever the page's CSS says. Where that can't be told from the page, the page
is left as it is, among other cases:

- when a script is left in it in any form a browser runs while it prints: a
  `<script>` (with `keep_scripts(true)`), an `on…` attribute, a `srcdoc`, a
  `javascript:` URL, or an `<iframe>`, `<object>` or `<embed>` that loads a
  document;
- when some of its CSS isn't in the page: a `<link rel="stylesheet">` or an
  `@import`. Sheets loaded through a `ResourceLoader` are in the page;
- when a `<style>` has a child that isn't text, or the page has a declarative
  shadow root (`<template shadowrootmode>`);
- when the pass runs out of its steps or of time. The plain page then goes out;
  the render doesn't fail.

A page that isn't in standards mode (no doctype, or an old one) gets only the
`<svg>` sizes. With `printer_safe(false)`, `page.html` is the serialized DOM and
nothing else.

The same options, HTML and files give the same bytes. Scripts see `now_ms` and
`utc_offset_minutes`, and `Math.random` follows `seed`. Timers run on a virtual
clock, so a `setTimeout` of a second costs no real time.

### External files and frames

Without a `ResourceLoader` a page loads nothing: no network and no files. A
`<script src>` or `<iframe src>` fails in the report, and a
`<link rel="stylesheet">` stays in the output for the printer to deal with.

`ResourceLoader::from_map` serves fixed bytes, with its URLs as the allowlist,
matched exactly. `ResourceLoader::new` takes an allowlist and a function:

```rust
use surimono::{Options, Renderer, ResourceLoader};

fn main() -> Result<(), surimono::Error> {
    let font = std::fs::read("fixtures/fonts/NotoSans-Regular.ttf").expect("the font");
    let loader = ResourceLoader::new(["/assets/"], |url| match url {
        "/assets/title.js" => Ok(b"document.title = 'Q3';".to_vec()),
        _ => Err("no such file".to_owned()),
    });
    let renderer = Renderer::new(Options::new().font(font).resource_loader(loader))?;
    let page = renderer.render_page(
        "<!doctype html><title></title><script src='/assets/title.js'></script>",
    )?;
    assert!(page.html.contains("<title>Q3</title>"));
    Ok(())
}
```

- **The URL is the attribute as written**, with no base applied.
- **The allowlist** lets through a URL equal to an entry, and with an entry
  ending in `/`, what is under it. Such an entry refuses paths that climb out of
  it, but a function that reads files should still check what it opens.
- **The function** must return the same bytes for the same URL, or the same input
  stops giving the same output. It runs once per URL, on a worker thread, one
  load at a time, and nothing interrupts it, so bound its own wait. After 10
  seconds of loads, or 1,000 of them, the rest fail in the report and the page
  goes on. Its `Err` message goes in the report too.
- **Scripts load before the first script runs**, and stylesheets after the
  scripts settle, where they count against the `time` for that step.
- **A stylesheet comes out as `<style>`**, so the printer doesn't need the file.
  A `url()` or `@import` in it isn't followed. A `<link>` off the list stays as
  it was; a listed one that can't be had goes from the page and into the report.
- **A frame** loads through the same function and list, with its `#fragment`
  taken off, and a failed one reports as `ResourceKind::Script`. A script can add
  a frame, so loads can happen while the page runs, under its time limit, and it
  can read back what it asked for: don't let through what a page mustn't see. A
  frame is a document with no browsing context: its scripts don't run, and a
  render has at most 64 frames.

`render_page_loading(html, allow, load, cancel)` takes a function for one render
instead, which runs on the calling thread. That's for a function that can't leave
that thread, such as a callback into another language.

### Layout queries

Nothing is laid out yet. Until phase 3 is done, `getBoundingClientRect`,
`offsetWidth`, `getComputedStyle` and the like answer zeros and empty strings
(`LayoutQueries::Zeros`), so a page that only reads them carries on, and one that
sizes something from them may draw it at size 0. If you would rather have no page
than a page built on those answers, use `LayoutQueries::Throw`:

```rust
use surimono::{Error, LayoutQueries, Options, Renderer};

fn main() -> Result<(), Error> {
    let font = std::fs::read("fixtures/fonts/NotoSans-Regular.ttf").expect("the font");
    let options = Options::new().font(font).layout_queries(LayoutQueries::Throw);
    let renderer = Renderer::new(options)?;
    let html = "<div id=d></div><script>try { d.offsetWidth } catch (e) {}</script>";
    match renderer.render_page(html) {
        Err(Error::PageLayoutQuery { member }) => assert_eq!(member, "offsetWidth"),
        other => panic!("{other:?}"),
    }
    Ok(())
}
```

The render ends at the first query, even if the page catches the exception.

When layout queries give real answers, they will measure as Chrome does, since
scripts are written against Chrome. fulgur lays pages out with Blitz, which
prints some of those layouts differently, so a page that sizes things from
those answers may print differently with stock fulgur.

### Cancelling a page

`render_page_cancellable` takes an `AtomicBool`, as `render_islands_cancellable`
does. Once it is true, a running script stops within milliseconds, as does the
parse before it and the work after it, the worker takes the next page, and the
call ends with `Error::Cancelled`. The flag doesn't carry into the next render.

A `ResourceLoader`'s function isn't interrupted: the call still ends at once, but
the worker is free only when the function returns. Under `render_page_loading`
the function runs on your thread, so the flag is looked at when it returns.

### What doesn't work yet

- A script a script inserts doesn't run, so script loaders do nothing.
- `type="module"` scripts don't run; `nomodule` scripts do.
- There is no `fetch`, `XMLHttpRequest`, `localStorage`, `Worker`, `WebSocket`,
  `matchMedia`, `ResizeObserver`, `IntersectionObserver`, `Intl` or shadow DOM.
  Using one stops that script, with an error in the report: a missing global
  gives `ReferenceError: fetch is not defined`, and calling a missing method
  (`element.attachShadow()`) gives `TypeError: not a function`.
- `document.write` doesn't exist, so the script that calls it stops there.
- `innerText` sees no stylesheet until phase 3: it follows `hidden`, an inline
  `display: none` and each element's default display, so text a stylesheet
  hides is still in it.
- `document.cookie` is empty, and a write does nothing.
- `focus()` sets `document.activeElement`, but `:focus` doesn't match.
- `customElements` is a registry only: `define`, `get` and `whenDefined` work,
  but a defined element is never constructed or upgraded, so its class's code
  doesn't run.
- `navigator` is fixed values, the same on every machine: user agent
  `Mozilla/5.0 (compatible; surimono)`, language `en-US`, an empty `platform`,
  `onLine` and `cookieEnabled` false, `hardwareConcurrency` 1.
- A `<canvas>` has no `getContext`; canvas drawing is planned for phase 3.
- Images aren't loaded for a page: no `load` or `error` event fires for one,
  `new Image()` included. `src` stays for the printer.

[docs/COMPAT.md](../COMPAT.md) has the full list, with the test behind each row.

### Printing pages you don't trust

The printer has no limits of its own. If pages come from people you don't trust,
print them in a process of your own, with its own timeout: a timeout can't stop a
thread, or catch what ends a process. Give it stack for the depth a page may
reach: at least 2 MiB, and 8 MiB leaves room. A Rust program's spawned threads
have 2 MiB unless you say otherwise.

A small binary does the printing, on a thread with that stack:

```rust
// src/bin/print-pdf.rs
use std::process::ExitCode;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const STACK_BYTES: usize = 8 * 1024 * 1024;

fn print(html: String, pdf: String, fonts: Vec<String>) -> Result<(), BoxError> {
    let mut assets = fulgur::AssetBundle::new();
    for path in fonts {
        assets.add_font_bytes(std::fs::read(path)?)?;
    }
    let engine = fulgur::Engine::builder().assets(assets).system_fonts(false).build();
    std::fs::write(pdf, engine.render(&std::fs::read_to_string(html)?)?)?;
    Ok(())
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (Some(html), Some(pdf)) = (args.next(), args.next()) else {
        eprintln!("usage: print-pdf page.html out.pdf font...");
        return ExitCode::from(2);
    };
    let fonts: Vec<String> = args.collect();
    let printer = std::thread::Builder::new()
        .stack_size(STACK_BYTES)
        .spawn(move || print(html, pdf, fonts));
    match printer.map(|thread| thread.join()) {
        Ok(Ok(Ok(()))) => ExitCode::SUCCESS,
        Ok(Ok(Err(e))) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
        _ => ExitCode::FAILURE,
    }
}
```

The application runs it with a timeout, and kills it at the timeout:

```rust
use std::io;
use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

pub fn print_in_child(printer: &str, args: &[&str], time: Duration) -> io::Result<ExitStatus> {
    let mut child = Command::new(printer).args(args).spawn()?;
    let deadline = Instant::now() + time;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            return Err(io::ErrorKind::TimedOut.into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
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
- `PageMemoryLimit`, `PageTimeLimit`, `PageStackOverflow`, `PageDomLimit`,
  `PageLayoutQuery` and `PageFailed` belong to a page: see
  [Report or error](#report-or-error). An exception in a page's script isn't an
  error at all.
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

`render_page` gives the same promise, with the same files from any loader.

Runnable versions of the island step and the page step, with their tests, are in
[`examples/rust-islands`](../../examples/rust-islands).
