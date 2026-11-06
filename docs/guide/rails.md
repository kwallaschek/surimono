# Rails with fulgur

This guide turns a Rails view with charts into a PDF without Chrome: the view
writes island markers, surimono draws them as SVG, and
[fulgur](https://github.com/fulgur-rs/fulgur) prints the HTML. The same steps
work with any engine that prints HTML with inline SVG. When a template's own
scripts build the page, surimono can run those too: see
[Page scripts](#page-scripts).

The fulgur parts follow fulgur's Ruby gem 0.40.0.

## Install

surimono isn't released yet: there is no gem on RubyGems, so
`gem install surimono` doesn't work, and the first release is planned for v0.3.
Build the gem from a clone instead. That needs Rust 1.89 or later, a C compiler
and libclang (rb-sys generates its Ruby bindings with bindgen).

```sh
git clone https://github.com/kwallaschek/surimono
cd surimono/bindings/ruby
bundle install
bundle exec rake compile   # builds the extension; expect several minutes
```

Then point your app's Gemfile at the clone:

```ruby
# Gemfile
gem "surimono", path: "/path/to/surimono/bindings/ruby"
gem "fulgur", "0.40.0"
```

Bundler doesn't compile a gem from `path:`, so run `rake compile` first, and
again after you pull. Each machine that runs the app needs the compiled clone.

surimono supports CRuby 3.2 to 4.0, and fulgur's gem 0.40.0 supports Ruby 3.1 to
3.4, so use Ruby 3.2, 3.3 or 3.4. fulgur publishes precompiled gems; check that
yours is among them.

*Planned for the first release, not available today:* `gem "surimono"` from
RubyGems, with precompiled native gems for Linux (x86_64 and aarch64, glibc) and
macOS (x86_64 and arm64), so `bundle install` needs no Rust toolchain there.

## Fonts

surimono measures and draws chart labels with font files, and fulgur prints the
page's own text with font files. Give both **the same files, in the same
order**, so charts and text match.

Keep the fonts in the app and list them once:

```ruby
# config/initializers/pdf_fonts.rb
PDF_FONTS = %w[
  NotoSans-Regular.ttf
  NotoSans-Bold.ttf
  NotoSansJP-Regular.otf
  NotoSansJP-Bold.otf
].map { |name| Rails.root.join("app/fonts", name).to_s }.freeze
```

- **List the files explicitly**, not with `Dir.glob`. The order decides which
  font a character falls back to, and a glob's order can differ between
  machines.
- **Use paths as Strings.** fulgur's `AssetBundle#add_font_file` takes a String
  path. surimono takes Strings, `Pathname`s, or font data read with
  `File.binread`.
- **TTF, OTF or TTC only.** surimono rejects WOFF and WOFF2. Convert web fonts to
  TTF or OTF first.

### Chart text is drawn as outlines

By default surimono draws the text in every chart as vector outlines from your
font files. fulgur 0.40 would otherwise print `<svg>` text with the server's
system fonts, not the ones you register, so labels could print in the wrong font
and overflow. Outlines print exactly as measured on every machine, but chart
labels can't be selected, searched or copied in the PDF. Each label becomes a
`<g role="img" aria-label="…">`, so the words stay in the SVG for
accessibility. The [islands guide](islands.md#chart-text-is-drawn-as-outlines)
has the details.

To keep real text, pass `text: :keep` to `Surimono::Renderer.new`. With fulgur
0.40 that prints chart labels in a system font, so only do it with an engine
that prints SVG text with your registered fonts.

### Name your fonts in CSS

surimono doesn't touch the rest of the page, and fulgur's Ruby gem 0.40.0 can't
turn system fonts off. Text styled with a generic family alone
(`font-family: sans-serif`) prints in whatever font the server happens to have:
one font on your Mac, another in a Linux container, or none. Name the family
inside the font file instead:

```css
html { font-family: "Noto Sans", "Noto Sans JP"; }
```

The family name is the one stored in the file, not the file name. On Linux,
`fc-scan --format "%{family}\n" app/fonts/NotoSansJP-Regular.otf` prints it.

## One renderer, shared

A `Surimono::Renderer` loads the fonts and keeps a pool of JavaScript runtimes
with the chart libraries loaded from precompiled bytecode. A `Fulgur::Engine`
holds its fonts too. Build each once and share it between threads. surimono
releases the GVL while it renders, so other threads keep running.

```ruby
# app/lib/pdf_renderer.rb
class PdfRenderer
  LOCK = Mutex.new

  # The shared renderer, built on first use.
  def self.instance
    @instance || LOCK.synchronize { @instance ||= new }
  end

  def self.render(html) = instance.render(html)
  def self.islands(html) = instance.islands(html)

  def initialize
    @surimono = Surimono::Renderer.new(
      fonts: PDF_FONTS,
      pool_size: Integer(ENV.fetch("RAILS_MAX_THREADS", 3)),
    )
    assets = Fulgur::AssetBundle.new
    PDF_FONTS.each { |path| assets.add_font_file(path) }
    @fulgur = Fulgur::Engine.new(assets: assets)
  end

  # HTML with island markers in, HTML with inline SVG out.
  def islands(html)
    @surimono.render(html)
  end

  # HTML with island markers in, Fulgur::Pdf out.
  def render(html)
    @fulgur.render_html(islands(html))
  end
end
```

Why it is built this way:

- **Build the fulgur engine after the fork.** A `Surimono::Renderer` survives
  `fork`: Puma with `preload_app!` or `fork_worker`, Unicorn, Resque and
  Sidekiq swarm can load the app, build the renderer, and fork. Its worker
  threads don't survive the fork, so the first render in each child starts that
  child's own runtime pool and text measurer, sharing the font data with the
  parent. fulgur makes no such promise, so build its engine in each worker,
  after the fork. The lazy `instance` above does that as long as nothing calls
  it before the fork; don't warm it up in an initializer.
- **`pool_size`** is the most islands this process renders at once. The pool
  starts with one JavaScript runtime and adds one only when renders overlap, up
  to `pool_size`, and a runtime idle for a couple of seconds frees its memory.
  Set it to the threads that render PDFs: Puma's thread count, or Sidekiq's
  `concurrency`. Extra renders wait for a free runtime. The default is the
  number of CPUs, at most 4; the most is 64. Each busy runtime can use up to
  the memory limit (64 MB by default), and its heap is freed after every
  island.
- **`Fulgur::Engine.new` takes the fonts out of the `AssetBundle`** you pass it,
  leaving the bundle empty. Build a new bundle for every engine.
- **Page size and margins** come from the template's `@page` rule below. If you
  pass `page_size:` or `margin:` to `Fulgur::Engine.new`, they override it.

`Surimono.render(html, fonts: PDF_FONTS)` is a shortcut that keeps one renderer
per set of options in the process (up to 4, least recently used dropped first).
It is fine for scripts and small apps. With several option sets or font data
passed as Strings, prefer an explicit renderer as above, since the shortcut
compares the options on every call, and a fifth option set rebuilds a
renderer.

### Dates and time zones

Scripts never see the server's clock or time zone. `Date.now()` and
`new Date()` return 2026-01-01 00:00:00 UTC, and local time is UTC, unless you
set them:

```ruby
Surimono::Renderer.new(fonts: PDF_FONTS, now: Time.utc(2026, 4, 1), utc_offset_minutes: 540)
```

`utc_offset_minutes: 540` shows time axes in Japan time. These options belong
to the renderer, so don't build one per document with that document's time;
put dates a chart shows into its spec instead, and set `now:` only for charts
that compute "today" themselves.

## The view

A small helper writes island markers safely:

```ruby
# app/helpers/surimono_helper.rb
module SurimonoHelper
  # <%= surimono_island "echarts", @sales_chart, width: 680, height: 320, id: "sales" %>
  def surimono_island(renderer, spec, width: 600, height: 400, **attributes)
    json = spec.is_a?(String) ? spec : spec.to_json
    tag.script(json_escape(json).html_safe,
               type: "application/vnd.surimono+json",
               data: { renderer: renderer, width: Integer(width), height: Integer(height) },
               **attributes)
  end
end
```

`json_escape` turns `<`, `>` and `&` into escapes such as `\u003c`, so a `</script>`
inside your data can't end the marker early. `html_safe` stops ERB from turning
`"` into `&quot;`, which would break the JSON: browsers and surimono read script
text exactly as written.

A layout for PDFs, with the CSS inline because fulgur doesn't fetch stylesheets
(or register them with `assets.add_css_file`):

```erb
<%# app/views/layouts/pdf.html.erb %>
<!doctype html>
<html lang="<%= I18n.locale %>">
<head>
  <meta charset="utf-8">
  <title><%= yield :title %></title>
  <style>
    @page { size: A4; margin: 15mm; }
    html { font-family: "Noto Sans", "Noto Sans JP"; font-size: 10.5pt; }
    .surimono-island > svg { display: block; }
  </style>
</head>
<body><%= yield %></body>
</html>
```

And the view:

```erb
<%# app/views/reports/show.html.erb %>
<% content_for :title, @report.title %>
<h1><%= @report.title %></h1>
<%= surimono_island "echarts", @report.sales_chart, width: 680, height: 320, id: "sales" %>
<p><%= @report.summary %></p>
```

`@report.sales_chart` returns an ECharts option as a Hash. A4 with 15 mm margins
leaves 680 CSS pixels of width; the [islands guide](islands.md#sizing) has other
sizes and the per-renderer notes.

## In a controller

For small documents, render in the request:

```ruby
class ReportsController < ApplicationController
  def show
    @report = Report.find(params[:id])
    html = render_to_string(:show, layout: "pdf", formats: [:html])
    pdf = PdfRenderer.render(html)
    send_data pdf.to_s, type: "application/pdf", disposition: "inline",
                        filename: "report-#{@report.id}.pdf"
  end
end
```

`pdf.to_s` is the PDF as a binary String. `Fulgur::Pdf` also has
`write_to_path`, `write_to_io` and `to_base64`.

## In a background job

Large or many documents belong in a job. `ApplicationController.render` renders
a view outside a request:

```ruby
class ReportPdfJob < ApplicationJob
  queue_as :pdf

  # Same input, same error: retrying doesn't help, so report and stop.
  discard_on(Surimono::Error) { |_job, error| Rails.error.report(error) }
  # A time limit can depend on how busy the machine is; try again later.
  retry_on Surimono::TimeLimitError, wait: 1.minute, attempts: 3

  def perform(report)
    html = ApplicationController.render(template: "reports/show", layout: "pdf",
                                        assigns: { report: report })
    pdf = PdfRenderer.render(html)
    report.pdf.attach(io: StringIO.new(pdf.to_s), filename: "report-#{report.id}.pdf",
                      content_type: "application/pdf")
  end
end
```

ActiveJob checks the handler declared last first, so `retry_on` must come after
`discard_on` for time-limit errors to be retried.

## Page scripts

Island markers suit charts you can describe as JSON. When the template's own
`<script>` elements build the page (filling tables, formatting numbers, drawing
with a library), `render_page` runs them as a browser would, on a virtual clock
until the page settles, then draws any island markers. Plain `render` never runs
a page's scripts.

```ruby
page = Surimono.render_page(html, fonts: PDF_FONTS)
page.html   # the page after its scripts ran, for fulgur
page.report # what happened while it ran
```

`page` is a `Surimono::Page`. In the app, add it to the shared renderer from
[One renderer, shared](#one-renderer-shared):

```ruby
# In PdfRenderer

  # A page with scripts in, the Surimono::Page out.
  def page(html, loader: nil, allow: nil)
    @surimono.render_page(html, loader: loader, allow: allow)
  end

  # A page with scripts in, Fulgur::Pdf out. Logs what went wrong in the page.
  def print_page(html)
    page = page(html)
    report = page.report
    if report.uncaught.positive? || report.unhandled_rejections.positive? || report.failed_count.positive?
      Rails.logger.warn("PDF page: #{report.to_h}")
    end
    @fulgur.render_html(page.html)
  end
```

`renderer.render(html, scripts: true)` is the same render, returning only
`page.html`; the report is lost.

### Report or raise

A page's own mistakes go in the report and the page goes on, as in a browser. A
limit raises, and there is no page.

| `page.report` | What it says |
|---|---|
| `settled` | Why it stopped: `:idle` (nothing left to run), `:virtual_budget` (timers left after 10 seconds of virtual time, as with a `setInterval` nobody clears) or `:task_limit` (100,000 tasks, with more due) |
| `uncaught`, `errors` | Exceptions nothing caught, and the first few as messages |
| `unhandled_rejections`, `rejections` | Promises rejected with no handler, and the first few reasons |
| `failed_count`, `failed` | External scripts, images and stylesheets the page didn't get, and the first few as `{ kind:, url:, reason: }` Hashes, `kind` being `:script`, `:image` or `:stylesheet` |

A report isn't always clean on a page that printed well: `:virtual_budget` is
normal for a page with a clock in it, and every `<img src>` is listed, because
surimono leaves images to the printer. Decide what your app treats as a failure;
the sample above only logs.

| Raised | When |
|---|---|
| `Surimono::MemoryLimitError` | The page's scripts used more than `limits[:memory_mb]` |
| `Surimono::TimeLimitError` | The page ran past `limits[:time_ms]`; see [Page limits](#page-limits) |
| `Surimono::StackOverflowError` | The page's scripts recursed deeper than `limits[:stack_kb]` allows, and nothing caught it |
| `Surimono::DomLimitError` | The page or its scripts passed a limit on the DOM (nodes, depth, text, attributes), or its CSS passed a limit, or its output is over `limits[:output_mb]`. The message says which. |
| `Surimono::LayoutQueryError` | With `layout_queries: :throw`, a script asked for layout; `#member` says through what |
| `Surimono::RenderError`, kind `"page"` | The page couldn't run, or used something the DOM defines and surimono doesn't do yet |

Islands in a page fail as they do anywhere, under `error_policy:`.

### What comes out

`page.html` is the document serialized from a parse, not your input with changes,
so it isn't byte-identical to what you passed in:

- Attributes are quoted, a missing `<head>` or `<body>` is added, and comments
  are gone.
- `<script>` and `<noscript>` are gone, except island markers on their way to
  becoming SVG. To keep the scripts, for an engine that would run them, pass
  `keep_scripts: true` to `Surimono::Renderer.new`.
- A page with no doctype or an old one comes out with no doctype;
  a standards-mode page comes out with `<!doctype html>`.
- The output is screened so the printer's parse matches ours.

The same HTML, options and files give the same bytes. Scripts see the clock in
[Dates and time zones](#dates-and-time-zones), and `Math.random` follows `seed:`.
Timers run on a virtual clock, so a `setTimeout` of a second costs no real time.

### External files

By default a page loads nothing: no network and no files. A `<script src>` or
`<iframe src>` fails in the report, and a `<link rel="stylesheet">` stays in the
output for the printer to deal with.

For a fixed set of files, pass `resources:` to the renderer. The keys are the
URLs exactly as the page writes them:

```ruby
renderer = Surimono::Renderer.new(
  fonts: PDF_FONTS,
  resources: {
    "/assets/invoice.js" => File.read("app/assets/invoice.js"),
    "/assets/page.css" => File.read("app/assets/page.css"),
  },
)
```

For a choice per render, pass a proc and an allowlist to `render_page`:

```ruby
loader = ->(url) { Rails.root.join("public", url.delete_prefix("/")).binread }
page = renderer.render_page(html, loader: loader, allow: ["/assets/"])
```

- **The url is the attribute as written**, with no base applied: `../a.js`
  arrives as `../a.js`, whatever `<base>` says. Rails' `javascript_include_tag`
  adds a digest and writes `/assets/invoice-3f2a….js`, and with an asset host a
  whole URL, so match what the page really has.
- **`allow:`** takes URLs that reach the proc: an entry equal to one, or an entry
  ending in `/` for what is under it. With no entry, nothing reaches it. An
  ending `/` refuses paths that climb out of it, but check what you open anyway.
- **The proc** returns a String, or `nil` if there is none, which goes in the
  report. An exception in it ends the render and is raised from `render_page`.
  It runs on your thread, one call at a time, in document order, once per URL.
  Nothing interrupts it, so give it a timeout of its own. After 10 seconds of
  loads, or 1,000 of them, the rest fail in the report and the page goes on.
  Stylesheets load after the scripts, and count against `time_ms` too.
- **Return the same bytes for the same URL**, or the same input stops giving the
  same output.
- **A stylesheet comes out as `<style>`**, so the printer doesn't need the file.
  A `url()` or `@import` in it isn't followed. A `<link>` not on the list stays
  as it was; a listed one that can't be had goes from the page and into the
  report.
- **A frame** (`<iframe src>`) loads through the same proc and list, with its
  `#fragment` taken off, and a failed one reports as `:script`. A script can add
  a frame and read what it loaded, so don't allow what a page mustn't see.

### Page limits

`limits:` applies to a page as one, not to each script in it:

- **`time_ms`** is one deadline for all the page's scripts together, timers
  included. Parsing the page has a limit of the same length, and so has
  everything after the scripts (serializing, stylesheets, the checks for the
  printer), so a page takes at most about three times `time_ms` before its
  islands, and each island then has its own `time_ms`. Time spent in your loader
  before the scripts is capped only by the 10 seconds above.
- **`memory_mb`** and **`stack_kb`** are one heap and one stack for all of them.
- **`output_mb`** (default 64, at least 1) caps the HTML `render_page` gives back
  and the text a page may hold while it runs. Past it, `DomLimitError`.

```ruby
Surimono::Renderer.new(fonts: PDF_FONTS, limits: { time_ms: 1_000, output_mb: 8 })
```

Interrupts work as they do for islands: `Timeout.timeout` raises at once, and the
page's script stops within milliseconds, which frees its runtime.

```ruby
page = Timeout.timeout(10) { PdfRenderer.instance.page(html) }
```

### Layout queries

Nothing is laid out yet. Until phase 3 is done, `getBoundingClientRect`,
`offsetWidth`, `getComputedStyle` and the like answer zeros and empty strings,
so a page that only reads them carries on, and one that sizes something from
them may draw it at size 0. If you would rather have no page than a page built
on those answers, ask for an error:

```ruby
Surimono::Renderer.new(fonts: PDF_FONTS, layout_queries: :throw)
```

The first query then raises `Surimono::LayoutQueryError`, even if the page
catches the exception.

### What doesn't work yet

- A script a script inserts doesn't run, so script loaders do nothing. Inline
  and `src` scripts in the HTML do.
- `type="module"` scripts don't run; `nomodule` scripts do.
- There is no `fetch`, `XMLHttpRequest`, `localStorage`, `Worker`, `WebSocket`,
  `matchMedia`, `ResizeObserver` or `IntersectionObserver`.
- There is no `navigator`, `URL`, `URLSearchParams`, `TextEncoder`, `atob`,
  `btoa`, `Intl`, `structuredClone`, `customElements` or shadow DOM.
- Many element helpers are missing, among them `innerText`, `focus`, a table's
  `insertRow` and `rows`, a `<select>`'s `options` and a form's `elements`.
  `textContent`, `createElement` and `append` work. Calling a missing method
  stops that script, with `TypeError: not a function` in the report.
- `document.write` doesn't exist, so the script that calls it stops there.
- A `<canvas>` has no `getContext`; canvas drawing is planned for phase 3.
- Images aren't loaded for a page: no `load` or `error` event fires for one, and
  there is no `Image` constructor. `src` stays for the printer.
- A frame's scripts don't run.

[docs/COMPAT.md](../COMPAT.md) has the full list, with the test behind each row.

### Printing pages you don't trust

The printer has no limits of its own. If pages come from people you don't trust,
print them in a process of your own, not under a timeout: a timeout can't stop a
thread, or catch what ends a process. Give that process stack for the depth a
page may reach: at least 2 MiB, and 8 MiB leaves room. Ruby's threads, which
Puma and Sidekiq run on, have 1 MiB unless `RUBY_THREAD_MACHINE_STACK_SIZE` says
otherwise.

A small script does the printing. A child process's main thread takes its stack
from `rlimit_stack`:

```ruby
# bin/print_pdf.rb
require "bundler/setup"
require "fulgur"

html_path, pdf_path, *fonts = ARGV
assets = Fulgur::AssetBundle.new
fonts.each { |path| assets.add_font_file(path) }
Fulgur::Engine.new(assets: assets).render_html(File.read(html_path, encoding: "UTF-8")).write_to_path(pdf_path)
```

The app runs it with a stack and a timeout, and kills it at the timeout:

```ruby
# app/lib/untrusted_printer.rb
require "rbconfig"
require "timeout"

class UntrustedPrinter
  class Failed < StandardError; end

  STACK_BYTES = 8 * 1024 * 1024

  def self.print(html_path, pdf_path, seconds: 30)
    pid = Process.spawn(RbConfig.ruby, Rails.root.join("bin/print_pdf.rb").to_s, html_path, pdf_path, *PDF_FONTS,
                        rlimit_stack: STACK_BYTES)
    begin
      _, status = Timeout.timeout(seconds) { Process.wait2(pid) }
    rescue Timeout::Error
      Process.kill("KILL", pid)
      Process.wait(pid)
      raise Failed, "the printer took more than #{seconds} s"
    end
    raise Failed, "the printer ended with #{status}" unless status.success?
  end
end
```

Run the page's scripts in the app or in that process, as you like: `render_page`
has the limits above either way.

## Errors

Everything surimono raises is a `Surimono::Error`, a `StandardError`:

| Class | When |
|---|---|
| `Surimono::MemoryLimitError` | An island's or a page's script used more than `limits[:memory_mb]` |
| `Surimono::TimeLimitError` | An island or a page ran longer than `limits[:time_ms]` |
| `Surimono::StackOverflowError` | An island's or a page's script recursed deeper than `limits[:stack_kb]` allows |
| `Surimono::DomLimitError` | A page passed a limit on the DOM, its CSS or `limits[:output_mb]` |
| `Surimono::LayoutQueryError` | With `layout_queries: :throw`, a page's script asked for layout; `#member` names what it used |
| `Surimono::JavaScriptError` | An island's script threw; `#js_stack` has the JavaScript stack |
| `Surimono::RenderError` | A malformed marker, an unknown renderer, unusable output, more islands than `max_islands` (kind `"too-many-islands"`), markup too ambiguous to rewrite, or a page that can't run (kind `"page"`) |
| `Surimono::ConfigurationError` | From `Renderer.new`: a font that can't be loaded, a limit out of range, an unknown renderer name, a bundle that can't be used |
| `Surimono::InternalError` | A bug in surimono itself, such as a panic during a render. The process keeps running; please report it with the message. |

Island errors also have `kind` (a stable name such as `"time-limit"`),
`island_index` (from 0), `island_id` and `renderer`, so a log line can say which
chart failed:

```ruby
rescue Surimono::Error => e
  Rails.logger.error("PDF chart failed: #{e.kind} in island #{e.island_index} " \
                     "(#{e.island_id || e.renderer}): #{e.message}")
  raise
```

A page's own errors have no island, so `island_index` is `nil` on them. An
exception in a page's script doesn't raise at all: see
[Report or raise](#report-or-raise).

Wrong argument types raise `ArgumentError`, and a missing font file raises
`Errno::ENOENT`. fulgur raises `Fulgur::RenderError` and `Fulgur::AssetError`,
both under `Fulgur::Error`.

By default the first failed island fails the whole render. To print the rest of
the document and leave an empty `<div class="surimono-island
surimono-island-error">` of the chart's size where the chart was, pass
`error_policy: :placeholder` to `Surimono::Renderer.new`. Its `data-error`
attribute holds the kind and a comment inside it holds the message.

## Limits

The defaults are 64 MB of JavaScript heap, 2 seconds and 512 KB of stack per
island. A page's scripts share one set: see [Page limits](#page-limits). They are generous for ECharts and Vega charts of report size. Raise them
for very dense charts, and lower the time limit when documents come from users:

```ruby
Surimono::Renderer.new(fonts: PDF_FONTS, limits: { memory_mb: 128, time_ms: 5_000 })
```

The time limit applies to each island, so a document with 20 islands can take up
to 20 times as long. Two options bound a whole document:

- **`max_islands:`** (default 1,000). A document with more islands raises
  `Surimono::RenderError` with kind `"too-many-islands"` before any island
  runs. With documents from users, set it to what your templates need:

  ```ruby
  Surimono::Renderer.new(fonts: PDF_FONTS, limits: { time_ms: 1_000 }, max_islands: 20)
  ```

  Then one document spends at most about 20 seconds running chart scripts.
- **Timeouts cancel.** `Timeout.timeout`, `Rack::Timeout`, `Thread#raise`,
  `Thread#kill` or an exception a signal handler raises in the rendering thread
  stop a render, its running island included, and raise as usual (for example
  `Timeout::Error`). So a timeout around the call works:

  ```ruby
  html = Timeout.timeout(10) { PdfRenderer.islands(html) }
  ```

  Loading a bundle is the exception: it isn't interrupted, and the interrupt
  lands when the load ends. Precompiled built-ins load in place. A built-in
  compiled from source on first use, or your own bundle compiled at a forked
  worker's first render, can take up to twice `time_ms` (one limit to compile,
  one to run it once), plus any wait for a free runtime.

The printer has no limits of its own: see
[Printing pages you don't trust](#printing-pages-you-dont-trust).

## Testing

Test the island pass on its own; it is fast and needs no PDF parsing:

```ruby
test "report renders its chart" do
  html = ApplicationController.render(template: "reports/show", layout: "pdf",
                                      assigns: { report: reports(:q3) })
  out = PdfRenderer.islands(html)
  assert_includes out, %(<div class="surimono-island" id="sales" data-renderer="echarts" ) +
                       %(style="width:680px;height:320px;overflow:hidden"><svg)
  refute_includes out, "application/vnd.surimono+json"
  assert_equal out, PdfRenderer.islands(html) # same input, same bytes
end
```

## Licences

The gem includes `THIRD_PARTY_LICENSES.txt` at its root: the licences and
notices of every Rust crate and JavaScript library (ECharts, Vega and their
dependencies) built into it. If you ship the gem to others, in a container
image or an installer, ship that file with it. To find it:

```ruby
File.join(Gem.loaded_specs.fetch("surimono").full_gem_path, "THIRD_PARTY_LICENSES.txt")
```

## When something looks wrong

| Symptom | Likely cause |
|---|---|
| `RenderError` "the script's text is not valid JSON" | The spec was HTML-escaped. Use the helper above, or `raw json_escape(…)`. |
| Page text in the wrong font, or blank | The CSS names a generic family, or a family name that isn't in the font files. See [Name your fonts in CSS](#name-your-fonts-in-css). |
| Japanese chart labels as boxes | No Japanese font is registered with surimono. |
| Japanese page text as boxes or in an odd font | No Japanese font is registered with fulgur, or the CSS doesn't name it. |
| Chart labels in the wrong font, overflowing or clipped | The renderer was built with `text: :keep`. Remove it so chart text is drawn as outlines. |
| Chart labels can't be selected or searched | Expected: chart text is drawn as outlines. See [above](#chart-text-is-drawn-as-outlines). |
| A gap under every chart | The SVG is inline. Add `.surimono-island > svg { display: block; }`. |
| A render takes far longer than `time_ms` | The limit is per island. Lower `max_islands:` or put a timeout around the call (see [Limits](#limits)). |
| The first chart in each process is much slower than later ones | The chart libraries aren't loading precompiled bytecode. Check that `Surimono.precompiled_builtins` equals `Surimono.builtin_renderers.size`; if it doesn't, `SURIMONO_PRECOMPILE=0` was set when the gem was built or the app started, or the gem's precompiled bytecode doesn't fit its engine (a broken build). |
| A table or chart a page's script should fill is empty | Check `page.report`. An exception stops that script, and a `<script src>` that didn't load is in `failed`. |
| A chart library draws nothing, or at size 0 | It measures the page, and layout queries answer zeros until phase 3. `layout_queries: :throw` shows the first query. Use an island for the chart. |
| `settled` is `:virtual_budget` | A timer was still set after 10 seconds of virtual time. Fine if the page had finished by then. |
| Scripts or styles from a CDN or `/assets/` are missing | Nothing loads by default. Pass `resources:` or `loader:` with `allow:`, with URLs as the page writes them. |
| `RenderError` "unknown renderer" | A typo in `data-renderer`, or the renderer isn't in `renderers:`. The message lists the enabled ones. |
