# Real-world pages printed: where each result comes from

`index.md`, `print-pages.json` and the per-page folders are written by
`fixtures/tools/print-pages.mjs --plain`, and `hostile-page-results.json` by
`surimono-page-check hostile`. This file is hand-written. Paths are relabelled after
the run (the output directory reads `print-pages-out/`), so no machine path is
committed. The PNGs are reduced to 256 colours (`convert -strip -dither None -colors 256`,
which leaves out ImageMagick's date chunks, so the same run gives the same bytes);
nothing else is touched after the run.

**This run.** 14 Nov 2026, on the fleet's Linux machine (Ubuntu 24.04.5, 2 cores,
poppler 24.02.0), against the committed references of Chromium 153.0.8010.12, with
unchanged references, metric and thresholds.

- `surimono-page-check` in release mode from branch `p2-cf-07` at 24d81671: main at
  a68123fb (after #180, #184 and #186) plus this task's changes to
  `fixtures/tools/` only (page-check's `--no-printer-safe` and `--keep-scripts`,
  print-pages.mjs's `--plain` and DOM check), so the library is main's. Default
  limits, fixture fonts only. Its sha256 is in every `score.json`.
- fulgur-cli 0.40.0 from crates.io (`cargo install --locked`), sha256 `002a6c4d…`.
- `de_DE.UTF-8` isn't installed here, so the second process sets the variables and
  doesn't change libc's locale, as in `fixtures/checkpoint-2`; `Asia/Tokyo` is in
  effect.
- The render times in `index.md` are one wall-clock run on the shared 2-core
  machine, not a measurement; `bench/` has those.
- Run twice into two directories: the same scores, the same output and PDF hashes,
  and the same committed images byte for byte; only paths and render times differ.

**Result.**

| | Pages that pass, of 6 |
|---|---|
| Printer-safe output (the default) | 4: `alpine-invoice`, `d3-charts`, `highlightjs-code`, `noscript-message` |
| `--no-printer-safe` (the page's DOM as it is) | 2: `alpine-invoice`, `highlightjs-code` |
| The floor (fulgur printing Chrome's own DOM) | 2: `alpine-invoice`, `highlightjs-code` |

- All six are byte-identical in both modes, twice in one process and twice in a
  fresh one.
- **The plain output is what Chrome's scripts left, and prints as Chrome's DOM
  does.** Each page's plain PDF is byte for byte its floor's PDF (`printed.sha256`
  in `print-pages.json`), so with printer-safe output off every failure is fulgur's.
  The plain outputs are byte for byte those of the run at e879ec95 (the same
  `sha256` as its `print-pages.json`), so nothing merged since, #184's reflections
  included, changed what these pages' scripts leave.
- **Output = Chrome's DOM: 5 of 6.** `render_page`'s output with the page's scripts
  kept equals Chrome's DOM in the form the floor prints (no `<noscript>`, linked
  sheets inlined) for every page but `katex-autorender`. That one differs from byte
  26043 of the normalized texts, and only in how the output writes an attribute: each
  of its 12 `<math>` elements comes out as
  `<math :xmlns="http://www.w3.org/1998/Math/MathML">` where Chrome's DOM has
  `xmlns`, and with the 12 colons taken out the texts are equal. KaTeX sets `xmlns`
  with `setAttribute`, so the DOM is right (`crates/dom/tests/real_world.rs` compares
  `Document::to_html` with Chrome's and passes): the facade's printer screen
  re-parses its output with html5ever, which gives `xmlns` on a foreign element an
  empty prefix, and writes that prefix and a colon back
  (`crates/surimono/src/print/tree.rs`). Nothing prints differently for it: the
  page's plain PDF is its floor's. An `xmlns` that the parser reads from the markup
  is another case, wrong in the DOM itself (its name is `:xmlns`, and
  `getAttribute('xmlns')` is null); none of the six pages has one in its markup.
  `docs/COMPAT.md` has a row for each.

**What changed since e879ec95: printer-safe output (#180, on by default).** The
printer-safe outputs differ from the plain ones on three pages, and so do their PDFs
from the floor's:

| Page | What printer-safe output writes | Gap it works around | Shape / colour / hue, plain → printer-safe |
|---|---|---|---|
| `d3-charts` | `<style>@namespace s url(http://www.w3.org/2000/svg);@layer{s\|svg[width="640"]:not(s\|svg *){width:640px}s\|svg[height="208"]:not(s\|svg *){height:208px}}</style>` at the start of `<head>` | B1, a block-level `<svg>` that ignores its size attributes | 100.0% / 100.0% / 100.0% → 0.0% / 0.0% / 0.0%, pass |
| `noscript-message` | `<style>@layer{.row{border-top-color:#d9e2ec}}</style>` at the start of `<head>` | F1, a single-side border printed in the top colour | 100.0% / 0.0% / 100.0% → 9.7% / 0.0% / 0.0%, pass |
| `marked-report` | its one `<pre>`'s last newline left out (one byte less) | B2, the extra line after a final newline | 67.5% / 84.8% / 100.0% → the same, still a fail |

On `alpine-invoice`, `highlightjs-code` and `katex-autorender` the printer-safe
output is the plain one, byte for byte (same `sha256` and `plain_sha256`).

**What is left, and whose it is.** The gap ids are the drafts' in `docs/upstream/`
(F fulgur, B blitz-dom, P parley); the reproducer scores are fulgur 0.40.0's in
`fixtures/printers/reproducers/reproducers.md`. In the printer matrix
(`fixtures/printers/`, at 9d363d0d, before printer-safe output) fulgur 0.41.0 printed
all six pages and their floors pixel for pixel as 0.40.0 does.

| Page | Score (shape / colour / hue) | What fulgur 0.40.0 does | Gaps | Without touching the printer |
|---|---|---|---|---|
| `katex-autorender` | 85.2% / 0.0% / 0.0% | KaTeX's `vertical-align` on inline-blocks, negative margins and inline-block struts don't lay out as in Chrome. `E = mc²`, an inline-block in a paragraph directly under `<body>` that wraps, isn't painted at all: that is fulgur's F5, not KaTeX's. `@font-face` isn't loaded, so the fonts come with `--font` | B4 (`B4-valign-inline-block.html` 96.2%, `B4-valign-span.html` 66.2%), F5 (`F5-inline-block-wrapped.html` 100.0%), B3 and F8 | None that keeps the DOM; F5's workaround changes Chrome's page breaks |
| `marked-report` | 67.5% / 84.8% / 100.0% | The first paragraph wraps at "the print" instead of "the print guide", list markers sit against the text, inline `code` has no background, `<em>` isn't slanted | F6 (`F6-content-width.html` 100.0%), F2 (91.1%), F4 (100.0%), F3 (56.0%); B2 is worked around | F6's root-width rewrite is conditional and not shipped; none for F2, F3, F4 |

Q-GFX-01's patched fulgur build scored `marked-report` 44.6% / 0.0% / 0.0% with F2,
F3, F4, B2 and F6 fixed, and `katex-autorender` 85.2% / 0.0% / 0.0% with F5 fixed
(the formula is back; its layout still needs B4): the "Scores" section of
`docs/upstream/fulgur-layout-gaps-0.40.md`.

**Hostile scripts.** 79 of 79 cases and steps end as the manifest says, through
`render_page` at 24d81671: no `Crash`, no `Hang`.
