# Checkpoint 3: try the gem on a template of your own

Five steps to run one of your own templates through page scripts, from a clone.
You need Ruby 3.2 to 3.4 (fulgur's gem stops at 3.4), Rust 1.89 or later, a C
compiler and libclang.

Plan on about 20 minutes, about 10 of them waiting for the first build.

| Step | Measured here | Note |
|---|---|---|
| 1. Build | 2 min 43 s for `rake compile`, 64 of 230 crates to build; clone, `bundle install` and fulgur a few seconds | 2 cores, Linux, Ruby 3.3.6. The other crates were already built, so a first build takes longer: 10 minutes is an estimate |
| 2. Run | under a minute | The script itself took 3 s on a sample invoice |
| 3. Read the report | a few minutes | |
| 4. Fix what's yours | depends on the template | |
| 5. Send back | a few minutes | |

## 1. Build

```sh
git clone https://github.com/kwallaschek/surimono
cd surimono/bindings/ruby
bundle install
bundle exec rake compile
cd ../..
gem install fulgur -v 0.40.0
```

Run the rest from the root of the clone.

## 2. Run one template

Save the HTML your app sends, before any script has run. In a Rails app:

```sh
bin/rails runner 'File.write("tmp/page.html", ApplicationController.render(template: "reports/show", layout: "pdf", assigns: { report: Report.last }))'
```

Or fetch it from a running server, with `curl -o tmp/page.html` and the page's
URL. Don't use "Save as" in a browser: that saves the page after its scripts ran.

Then run [`examples/checkpoint-3/try_template.rb`](../../examples/checkpoint-3/try_template.rb)
on it. `--assets` is the folder that URLs starting with `/` come from:

```sh
ruby -Ibindings/ruby/lib examples/checkpoint-3/try_template.rb /path/to/app/tmp/page.html \
  --assets /path/to/app/public
```

It uses Noto Sans and Noto Sans JP from `fixtures/fonts`. If your CSS names other
fonts, add `--font file.ttf` for each, in fallback order. It writes
`page.out.html` and `page.pdf` next to `page.html`.

## 3. Read the report

The sample invoice, `show.html`, printed this:

```text
rendered in 85 ms, 16974 bytes
settled: idle
uncaught exceptions: 0
unhandled rejections: 1
  Error: no csrf token
not loaded: 1
  script https://cdn.example.com/analytics.js: not in the loader's allowlist
layout queries: yes, first offsetWidth (answered with zeros until phase 3)
wrote show.out.html
wrote show.pdf
```

| Line | What to look at |
|---|---|
| `settled` | `idle` means everything ran. `virtual_budget` means timers were still set after 10 seconds of virtual time, which is fine if the page had finished by then. `task_limit` means a loop of timers that never ends. |
| `uncaught exceptions` | Each one stopped one script, and the rest of the page went on. Does the PDF miss what that script did? |
| `unhandled rejections` | A promise failed with nothing to catch it. Same question. |
| `not loaded` | Files the page didn't get. An analytics script doesn't matter; your app's own script does. Every image is listed here too, because surimono leaves them to fulgur. |
| `layout queries` | The script asked for sizes and got zeros. Anything it sizes from them may be missing or at size 0. That needs phase 3. |

Then open `page.pdf` next to what Grover prints for the same page.

## 4. Common failures

| You see | Cause | Do this |
|---|---|---|
| `Unable to find libclang` during `rake compile` | rb-sys needs libclang | Linux: install `libclang-dev`. macOS: `xcode-select --install`. |
| `cannot load such file -- surimono/surimono` | The extension isn't built, or you aren't in the root of the clone | Run step 1, and run the script from the root |
| `no fulgur gem, so no PDF` | fulgur isn't installed for this Ruby | `gem install fulgur -v 0.40.0`, on Ruby 3.4 or older |
| `FAILED: Surimono::TimeLimitError` | The page's scripts took longer than 2 s together | Add `--time-ms 10000`. If it fails again, a script may never end. |
| `FAILED: Surimono::MemoryLimitError` or `StackOverflowError` | The scripts passed the 64 MB heap or the stack limit | Send this back, with the name of the library if you know it |
| `not in the loader's allowlist` | The URL isn't under `/`, such as a CDN or a relative path | Copy the file under `--assets`, and write its URL with a leading `/` |
| `the loader failed: the loader gave nothing` | `--assets` is the wrong folder, or the file isn't built | Check that the URL exists under that folder. With Rails, run `bin/rails assets:precompile` first. |
| `ReferenceError: fetch is not defined` | The platform feature doesn't exist here yet | Send this back. The same goes for `XMLHttpRequest`, `localStorage`, `matchMedia` and the others in the [Rails guide](rails.md#what-doesnt-work-yet). |
| `TypeError: not a function` | A method doesn't exist here yet, such as `document.write` or `attachShadow` | Send this back |
| No styles in the PDF | The stylesheet didn't load, so a `<link>` is left in `page.out.html`, which fulgur doesn't fetch | Add `--assets`, or fix its URL |
| Text in the wrong font, blank or as boxes | The CSS names a generic family, or a font not passed in | [Name your fonts in CSS](rails.md#name-your-fonts-in-css), and add `--font` |
| A chart is empty, or at size 0 | It measured the page | Expected until phase 3. Use an island: [the Rails guide](rails.md#the-view) shows the helper. |

## 5. What to send back

For each template you tried:

1. The script's output, as it printed.
2. `page.pdf` and Grover's PDF of the same page, or a note of what differs. Use
   test data, or leave them out if the page holds real customer data.
3. Which libraries the template loads, with versions if you know them.
4. One line: would this PDF ship as it is? If not, what stops it?

The script's output alone is useful, so send that even if the rest takes too
long.
