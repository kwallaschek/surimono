# Examples

Each example runs in CI through [`run.sh`](run.sh), so what you read here works.

| Example | What it shows |
|---|---|
| [`rust-islands/`](rust-islands) | A small binary: build a `Renderer` with fonts and a bundle of your own, render a report's islands, report a failed island clearly. Its tests run it as a user would. |
| [`rails-snippet/`](rails-snippet) | The [Rails guide](../docs/guide/rails.md) in plain Ruby: an ERB view with a safe island helper, one shared renderer that survives `fork`, and fulgur when the gem is installed. |
| [`shared/`](shared) | The report both examples render, and `sparkline.js`, a renderer written as a [bundle of your own](../docs/guide/islands.md#your-own-renderer). |

From the repository root:

```sh
cargo run -p surimono-example-rust-islands               # HTML with inline SVG to stdout
(cd bindings/ruby && bundle exec rake compile:dev)
ruby -Ibindings/ruby/lib examples/rails-snippet/render_report.rb out.pdf   # needs the fulgur gem
examples/run.sh                                          # everything CI checks
```

Both examples use the test fonts in `fixtures/fonts/` (Noto Sans and Noto Sans
JP, under the SIL Open Font License). Use the font files your PDF engine prints
with in your own app.

`run.sh` also renders `shared/report.html` through both bindings and checks
that the bytes are identical: the same input and options give the same output,
whichever language calls surimono.
