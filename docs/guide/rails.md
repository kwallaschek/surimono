# Rails with fulgur

This guide turns a Rails view with charts into a PDF without Chrome: the view
writes island markers, surimono draws them as SVG, and
[fulgur](https://github.com/fulgur-rs/fulgur) prints the HTML. The same steps
work with any engine that prints HTML with inline SVG.

The fulgur parts follow fulgur's Ruby gem 0.40.0.

## Install

```ruby
# Gemfile
gem "surimono"
gem "fulgur"
```

surimono supports CRuby 3.2 to 4.0, and fulgur's gem 0.40.0 supports Ruby 3.1 to
3.4, so use Ruby 3.2, 3.3 or 3.4. surimono ships precompiled native gems for
Linux (x86_64 and aarch64, glibc) and macOS (x86_64 and arm64), so
`bundle install` needs no Rust toolchain there. fulgur publishes precompiled
gems too; check that yours is among them.

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

## Errors

Everything surimono raises is a `Surimono::Error`, a `StandardError`:

| Class | When |
|---|---|
| `Surimono::MemoryLimitError` | An island's script used more than `limits[:memory_mb]` |
| `Surimono::TimeLimitError` | An island's script ran longer than `limits[:time_ms]` |
| `Surimono::StackOverflowError` | An island's script recursed deeper than `limits[:stack_kb]` allows |
| `Surimono::JavaScriptError` | An island's script threw; `#js_stack` has the JavaScript stack |
| `Surimono::RenderError` | A malformed marker, an unknown renderer, unusable output, more islands than `max_islands` (kind `"too-many-islands"`), or markup too ambiguous to rewrite |
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
island. They are generous for ECharts and Vega charts of report size. Raise them
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
  stop a render before its next island and raise as usual (for example `Timeout::Error`); the island already
  running ends within its own `time_ms`. So a timeout around the call works:

  ```ruby
  html = Timeout.timeout(10) { PdfRenderer.islands(html) }
  ```

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
| `RenderError` "unknown renderer" | A typo in `data-renderer`, or the renderer isn't in `renderers:`. The message lists the enabled ones. |
