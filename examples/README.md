# Examples

[`run.sh`](run.sh) runs the examples and the cross-binding checks below, and CI
runs it:

- on every pull request and push to `main`, the Linux job (Ruby 3.4) runs all
  of it as part of [`scripts/ci.sh`](../scripts/ci.sh);
- on `main`, nightly and release branches, the Ruby jobs (Ruby 3.2 to 4.0) run
  all of it again against the release build of the extension
  ([`scripts/ci/ruby.sh`](../scripts/ci/ruby.sh));
- on the same branches, the macOS and aarch64 Linux test jobs run the Rust
  example's tests (`cargo test --workspace` in
  [`scripts/ci/test.sh`](../scripts/ci/test.sh)) but not `run.sh`, so the Rails
  snippet and the cross-binding checks run on Linux x86_64 only.

Locally, `scripts/ci.sh --no-ruby` runs only the Rust example
(`examples/run.sh --no-ruby`).

| Example | What it shows |
|---|---|
| [`rust-islands/`](rust-islands) | Two small binaries. The default one builds a `Renderer` with fonts and a bundle of your own, renders a report's islands and reports a failed island clearly. `page` runs a page's own scripts with a resource loader and prints its report. Their tests run them as a user would. |
| [`rails-snippet/`](rails-snippet) | The [Rails guide](../docs/guide/rails.md) in plain Ruby. `render_report.rb`: an ERB view with a safe island helper, one shared renderer that survives `fork`, and fulgur when the gem is installed. `render_page.rb`: a page's own scripts, a loader and an allowlist, and the report. |
| [`checkpoint-3/`](checkpoint-3) | `try_template.rb`, the one-file script of the [checkpoint 3 checklist](../docs/guide/checkpoint-3.md): one saved page in, the report, HTML and a PDF out. |
| [`shared/`](shared) | What the examples render: the report and `sparkline.js`, a renderer written as a [bundle of your own](../docs/guide/islands.md#your-own-renderer), and `page.html`, an invoice whose script in `assets/` fills it in and adds a chart. |

From the repository root:

```sh
cargo run -p surimono-example-rust-islands               # HTML with inline SVG to stdout
cargo run -p surimono-example-rust-islands --bin page    # a page's scripts run; its report on stderr
(cd bindings/ruby && bundle exec rake compile:dev)
ruby -Ibindings/ruby/lib examples/rails-snippet/render_report.rb out.pdf   # needs the fulgur gem (Ruby 3.4 or older)
ruby -Ibindings/ruby/lib examples/rails-snippet/render_page.rb out.pdf     # the same, for the page
examples/run.sh                                          # the examples and the cross-binding checks
```

Both examples use the test fonts in `fixtures/fonts/` (Noto Sans and Noto Sans
JP, under the SIL Open Font License). Use the font files your PDF engine prints
with in your own app.

`run.sh` also renders `shared/report.html` and `shared/page.html` through both
bindings and checks that the bytes are identical: the same input and options
give the same output, whichever language calls surimono.
