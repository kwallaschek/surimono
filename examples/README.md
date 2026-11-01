# Examples

[`run.sh`](run.sh) runs both examples and the cross-binding check below, and
CI runs it:

- on every pull request and push to `main`, the Linux job (Ruby 3.4) runs all
  of it as part of [`scripts/ci.sh`](../scripts/ci.sh);
- on `main`, nightly and release branches, the Ruby jobs (Ruby 3.2 to 4.0) run
  all of it again against the release build of the extension
  ([`scripts/ci/ruby.sh`](../scripts/ci/ruby.sh));
- on the same branches, the macOS and aarch64 Linux test jobs run the Rust
  example's tests (`cargo test --workspace` in
  [`scripts/ci/test.sh`](../scripts/ci/test.sh)) but not `run.sh`, so the Rails
  snippet and the cross-binding check run on Linux x86_64 only.

Locally, `scripts/ci.sh --no-ruby` runs only the Rust example
(`examples/run.sh --no-ruby`).

| Example | What it shows |
|---|---|
| [`rust-islands/`](rust-islands) | A small binary: build a `Renderer` with fonts and a bundle of your own, render a report's islands, report a failed island clearly. Its tests run it as a user would. |
| [`rails-snippet/`](rails-snippet) | The [Rails guide](../docs/guide/rails.md) in plain Ruby: an ERB view with a safe island helper, one shared renderer that survives `fork`, and fulgur when the gem is installed. |
| [`shared/`](shared) | The report both examples render, and `sparkline.js`, a renderer written as a [bundle of your own](../docs/guide/islands.md#your-own-renderer). |

From the repository root:

```sh
cargo run -p surimono-example-rust-islands               # HTML with inline SVG to stdout
(cd bindings/ruby && bundle exec rake compile:dev)
ruby -Ibindings/ruby/lib examples/rails-snippet/render_report.rb out.pdf   # needs the fulgur gem (Ruby 3.4 or older)
examples/run.sh                                          # both examples and the cross-binding check
```

Both examples use the test fonts in `fixtures/fonts/` (Noto Sans and Noto Sans
JP, under the SIL Open Font License). Use the font files your PDF engine prints
with in your own app.

`run.sh` also renders `shared/report.html` through both bindings and checks
that the bytes are identical: the same input and options give the same output,
whichever language calls surimono.
