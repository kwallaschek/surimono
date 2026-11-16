# The printer matrix: where each result comes from

`index.md` and `printers.json` are written by `fixtures/tools/printers.mjs`, and
`reproducers/reproducers.md`, `reproducers.json` and the images beside them by
`fixtures/tools/reproducers.mjs`. This file is hand-written. The tools relabel paths
themselves, so no machine path is committed. The PNGs are reduced to 256 colours
(`convert -dither None -colors 256`); nothing else is touched after a run.

**This run.** 16 Nov 2026, on the fleet's Linux machine (Ubuntu 24.04, 2 cores,
poppler 24.02.0), against the committed references of Chromium 153.0.8010.12, with
unchanged references, metric and thresholds (Q-CF-04, after #200).

- **Printers.** The same three binaries as the 12 Nov run, by sha256
  (`printers.json`): fulgur-cli 0.40.0 (our pin) and 0.41.0, each from crates.io with
  `cargo install fulgur-cli --version =<version> --locked --root <its own root>` and
  rustc 1.97.0 (MIT OR Apache-2.0, https://github.com/fulgur-rs/fulgur); and
  `fixtures/tools/raikiri-print` in release mode with its committed `Cargo.lock` and
  rustc 1.89.0, on the library crate `fulgur-raikiri` 0.41.0 from crates.io (MIT OR
  Apache-2.0, the same repository, "Raikiri layout backend under development for
  fulgur"). The published fulgur CLI has no option that selects Raikiri, which is why
  there is a driver.
- **surimono.** The CLI and `surimono-page-check` in release mode from main at
  bf2ee568 (after #200), default limits. Every printer was given the same bytes for
  every page (`surimono_outputs_sha256`).
- **New in this run: the island fixture pages through the page path.** Each island
  fixture page is printed twice, from `render_islands` ("surimono's page", as before)
  and through `render_page` ("page path", `checkpoint.mjs --scripts`), which has the
  printer-safe output; `fixtures/README.md`, "The printer matrix", says why both rows
  stay. On all 11 fixtures the two outputs differ in the whitespace the page path's
  serializer writes (two newlines after the doctype and `<html>` fewer, two more
  before `</body>`), and on the five Japanese ones also in
  `<style>@layer{:root{text-autospace:no-autospace}}</style>` at the start of
  `<head>` (#200). fulgur 0.40.0 and 0.41.0 print each page path to the very PDF of
  its `render_islands` page, on all 11; Raikiri does so on the six Latin pages.
- **Command.** `printers.mjs --runs 5 --printer 0.40.0=… --printer 0.41.0=…
  --printer raikiri=…`, started while holding the fleet's build lock, so no build ran
  beside the timed prints. Other agents' tests could. The reproducers weren't run
  again: `reproducers/` is still the 12 Nov run, and nothing it prints changed.
- `de_DE.UTF-8` isn't installed here, as in `fixtures/checkpoint-3`.
- No printer failed to print a page: every cell has a score.

**Result.**

| | fulgur 0.40.0 | fulgur 0.41.0 | Raikiri (fulgur-raikiri 0.41.0) |
|---|---|---|---|
| Real-world pages that pass, of 6 (printer-safe output) | 4 | 4 | 5 |
| Their floors (Chrome's own DOM) that pass, of 6 | 2 | 2 | 5 |
| Island fixture pages that pass, of 11, from `render_islands` | 11 | 11 | 6 |
| The same pages through the page path (`render_page`), of 11 | 11 | 11 | **11** |
| Islands alone that pass, of 12 | 11 | 11 | 11 |
| Reproducers that pass, of 13 (12 Nov run) | 2 | 2 | 8 |

- **Raikiri on the five Japanese island pages, shape (the worst window):**

  | Fixture | `render_islands` | Page path | fulgur 0.40.0, either road |
  |---|---|---|---|
  | `echarts-bar-simple-ja` | FAIL 25.7% | pass 0.0% | pass 0.0% |
  | `echarts-mixed-dense-ja` | FAIL 22.0% | pass 0.0% | pass 0.0% |
  | `echarts-pie-ja` | FAIL 61.8% | pass 2.1% | pass 2.1% |
  | `vegalite-layered-dense-ja` | FAIL 52.6% | pass 0.0% | pass 0.0% |
  | `vegalite-line-ja` | FAIL 34.6% | pass 7.3% | pass 7.3% |

  This is the #200 engineer's measurement, reproduced to the tenth of a percent:
  `text-autospace: no-autospace` closes R1 (below) on every one, and Raikiri's page
  path scores equal fulgur's. The `render_islands` column is the 12 Nov run's, to the
  same tenth: that road has no printer-safe pass, so nothing changed for it.
- **fulgur 0.40.0 and 0.41.0** print the 22 island fixture pages to the same PDFs
  with and without the rule (fulgur ignores the property), and 0.41.0 reads the same
  as 0.40.0 in all 46 cells (`Cells that read differently`: 0). The 12 island-alone
  PDFs are byte for byte the same in both versions and the 34 page PDFs aren't, as on
  12 Nov, where the difference was traced to the `/Lang` and `dc:language` entries
  0.41.0 writes (not checked again here).
- **Real-world pages.** fulgur now passes 4 of 6 where the 12 Nov run had 2: that
  run printed main before printer-safe output (#180); `d3-charts` and
  `noscript-message` pass with it, as `fixtures/checkpoint-3` says. Raikiri passes 5 of
  6 on surimono's output and on Chrome's own DOM, as on 12 Nov; `marked-report` fails
  with all three (67.5% with fulgur, 72.1% with Raikiri). The `text-autospace` rule
  on `alpine-invoice` and `marked-report` changes no PDF: Raikiri prints those two
  outputs to the same files as the 12 Nov run's outputs without it.
- The 12 islands alone are the same pixels with all three printers, as before.

**The gaps of `docs/upstream` with Raikiri.** Judged on the reproducers
(`reproducers/reproducers.md`: Chrome prints each page on that run and each printer's
page is scored against it) and on the fixture pages above. Scores are shape, the
worst window; a reproducer's image shows Chrome, 0.40.0, 0.41.0 and Raikiri.

| Gap | Reproducer, fulgur 0.40.0 | Reproducer, Raikiri | On the fixture pages | With Raikiri |
|---|---|---|---|---|
| F1 one border colour for all sides | FAIL 100.0% | pass 0.0% | `noscript-message` 100% to 0.0% | closed |
| F2 list markers | FAIL 91.1% | FAIL 71.4% | `marked-report` | open, differently: the marker no longer touches the text, but `circle` and `square` print as the same small disc, smaller than Chrome's, and a nested list gets space above and below that Chrome doesn't give it |
| F3 synthesized italic | FAIL 56.0% | FAIL 18.8% | `marked-report`: `<em>` is upright | open: bold is the bold face, italic isn't synthesized |
| F4 inline backgrounds | FAIL 100.0% (colour 100.0%, hue 89.0%) | FAIL 0.0% (hue 59.1%) | `marked-report`: inline `code` has its background | closed for the background; the 1 px border of an inline box prints in another colour than Chrome's red, which is what fails the hue check |
| F5 inline-block in a wrapped paragraph | FAIL 100.0% | pass 2.3% | `katex-autorender`: `E = mc²` is on the page | closed |
| F6 content width cut to whole pixels | FAIL 100.0% | pass 0.0% | `marked-report`: the first paragraph still breaks one word earlier than in Chrome ("the print" / "guide"), so something else in the line's width differs | closed on the reproducer |
| F10 body-level paragraph never split | FAIL 62.6% | pass 0.0%, 2 pages as Chrome | no fixture page has one | closed |
| B1 block `<svg>` size | FAIL 100.0% | pass 0.0% | `d3-charts` 100% to 0.0% | closed |
| B2 empty line after a forced break | FAIL 16.9% | pass 0.0% | `marked-report`'s `<pre>` | closed |
| B4 inline-level layout (KaTeX) | FAIL 96.2% (`vertical-align` on inline-blocks) and 66.2% (on spans, `<sup>`, `<sub>`) | pass 0.0% and FAIL 62.2% | `katex-autorender` 85.2% to 0.0% | closed on the page; in the second reproducer `<sup>` and the lengths are raised and lowered, `<sub>` stays near the baseline and the lines sit a few pixels higher |
| R1 space between Japanese and Latin letters or digits (new with Raikiri) | pass 0.4% | FAIL 49.2%; pass 0.4% with `text-autospace: no-autospace` on the paragraph | the five Japanese island pages fail from `render_islands`, 22.0% to 61.8%, and pass through the page path, 0.0% to 7.3% (16 Nov) | closed by printer-safe output (#200), which gives a page with Japanese text Chromium 153's own `no-autospace`: Raikiri puts space between "2026" and "年" as CSS Text 4's initial `text-autospace: normal` says, Chromium 153 puts none. Open for `render_islands`, which has no printer-safe pass |

Seven of the ten gaps are closed on their reproducer or their page, F2, F3 and half
of F4 are open, and R1, new with Raikiri, is closed through the page path. F7 to F9 and B3, B5, B6 have no reproducer here.

**Time and memory.** One process per page, on 34 prints: the 11 island fixture pages
from each road and the 6 real-world pages and their floors. The three printers take
turns on each page: a round prints it once with each, one round isn't measured, then
come 5 timed rounds and 5 sampled ones, and who starts moves on every round and every
page (`print_order` in `printers.json`). Every figure here is in `index.md` or follows
from the single runs in `printers.json`.

| | fulgur 0.40.0 | fulgur 0.41.0 | Raikiri |
|---|---|---|---|
| Print time, median over the pages | 36.3 ms | 35.7 ms | 40.0 ms |
| Print time, sum of the pages' medians | 1293 ms | 1292 ms | 1921 ms |
| Peak PSS, median over the pages | 43.7 MB | 45.0 MB | 54.0 MB |
| Peak PSS, largest median of a page | 51.1 MB | 52.2 MB | 76.2 MB |

- **Raikiri, time.** Higher than 0.40.0's median on 29 of the 34 prints, lower on 5;
  the difference per page runs from −8.7 ms (`marked-report`) to +160.0 ms
  (`katex-autorender`) and is +5.4 ms at the median page; on 14 pages every run of
  Raikiri is slower than every run of 0.40.0, on none faster.
- **Raikiri, memory.** Higher on all 34, by +1.6 MB (`marked-report`) to +28.9 MB
  (`vegalite-scatter-dense-latin`), +16.1 MB at the median page, and every sampled run
  above every run of 0.40.0 on all 34.
- **fulgur 0.41.0 against 0.40.0.** Time: lower on 11, equal on 5, higher on 18;
  −5.4 ms to +3.3 ms per page, +0.1 ms at the median; on one page every run is slower.
  Memory: higher on all 34, by +0.9 to +2.4 MB, +1.4 MB at the median page, every run
  above on 31.
- **Against the 12 Nov run.** Every printer's medians are higher here (0.40.0's median
  print 36.3 ms against 26.7 ms then), on the same binaries and nearly the same pages:
  the machine was busier (other agents' tests run without the lock). The comparisons
  between printers within a run are what holds; the absolute times of the two runs
  don't compare. The 12 Nov run's own figures are in git history (`printers.json` at
  48dd396b).

**What Raikiri is told, and what it can't be.** `raikiri-print` takes the command
line the matrix gives fulgur and passes on all of it.

- **Fonts.** The files given with `--font` go in as an asset bundle, and system
  fonts are switched off (`RenderOptions::system_fonts: false`; the library refuses
  that without a bundled font). That is stricter than the fulgur column, whose CLI
  can't switch system fonts off and is kept from them by a fontconfig file.
- **Page size and margins.** The library's default configuration, A4 with its
  default margin, which a page's own `@page` overrides: the same as `fulgur render`
  without `--size` and `--margin`. Every fixture page and reproducer sets `@page`,
  and Raikiri's pages have Chrome's size in every cell.
- **Files.** Like fulgur, it reads only from the input file's folder, so the
  fixtures' `@font-face` URLs to `../../fonts` aren't followed and the fonts come
  from `--font`.
- **Not reachable.** The layout's own settings: the crate lays out with
  `LayoutConfig::default()` and offers no way to pass another. Nothing the matrix
  needed was missing.

**Raikiri's dependencies** (`fixtures/tools/raikiri-print/Cargo.lock`): the
`raikiri-*` crates 0.0.6 (https://github.com/fulgur-rs/raikiri), `shodo` 0.0.38 for
inline layout (https://github.com/fulgur-rs/shodo) with harfrust 0.14.0, fontique
0.11.1, skrifa 0.48.0 and icu_segmenter 2.4.0, taffy 0.15.0 for blocks, html5ever
0.40.1, selectors 0.40.0 and cssparser 0.37.0 for style, krilla 0.7.0 for the PDF,
usvg and resvg 0.48.1. There is no Blitz, no stylo and no Parley in it. Text is
shaped by harfrust 0.14.0 and broken into lines by shodo, where fulgur's Blitz
backend and surimono's measurements use Parley 0.6.0 with harfrust 0.3.2.

**The gaps of `checkpoint-3/NOTES.md`.** fulgur 0.41.0 closes none and opens none:
each failing page is the same pixels as with 0.40.0. With Raikiri `d3-charts`,
`katex-autorender` and `noscript-message` pass, and `marked-report` fails on its
list markers, its upright `<em>` and a line that breaks one word early.
`vegalite-line-ja`'s island alone fails the shape check at 10.2% with all three
(`docs/COMPAT.md`).

**fulgur's dependencies against our pins.** From each version's own `Cargo.lock`,
which `--locked` builds with, and the repository's `Cargo.lock`.

| Crate | fulgur-cli 0.40.0 | fulgur-cli 0.41.0 | surimono |
|---|---|---|---|
| blitz-dom | 0.2.4 | 0.2.4 | 0.2.4 (pinned) |
| blitz-html, blitz-traits | 0.2.0 | 0.2.0 | 0.2.0 (pinned) |
| stylo, stylo_dom, stylo_traits | 0.8.0 | 0.8.0 | 0.8.0 (pinned) |
| selectors, servo_arc | 0.32.0, 0.4.3 | 0.32.0, 0.4.3 | 0.32.0, 0.4.3 |
| taffy | 0.9.2 | 0.9.2 | 0.9.2 |
| parley, fontique | 0.6.0 | 0.6.0 | 0.6.0 (parley pinned) |
| harfrust (the shaper) | 0.3.2 | 0.3.2 | 0.3.2 |
| skrifa under Parley | 0.37.0 | 0.37.0 | 0.37.0 (pinned) |
| swash under Parley | 0.2.7 | 0.2.7 | **0.2.10** |
| html5ever | 0.35.0 | 0.35.0 | 0.35.0 |
| icu_segmenter under stylo, and the ICU4X crates with it | 2.2.0 | **2.3.0** (icu_provider 2.3.1) | 2.2.0 |
| A second skrifa and read-fonts | 0.44.0, 0.41.0 (fulgur's own) | 0.47.0, 0.44.0 (fulgur-blitz's own) | 0.44.0, 0.41.0 (under swash 0.2.10) |
| wuff (blitz-dom's WOFF decoder) | 0.2.3 | 0.2.9 | not built |
| krilla, krilla-svg | 0.7.0 | 0.7.0 | not used |
| usvg, resvg, fontdb | 0.45.1, 0.45.1, 0.23.0 | the same | not used |

- Blitz, stylo, Taffy, Parley, fontique and the shaper are the same in 0.41.0 as in
  0.40.0 and as our pins. Moving the pin to 0.41.0 changes none of them.
- 0.41.0 is three crates where 0.40.0 was one: `fulgur` re-exports `fulgur-blitz`
  (layout and drawing, on Blitz), which sits on `fulgur-core` (configuration, assets,
  PDF utilities). Its minimum Rust is 1.89.0, our toolchain.
- Two differences sit in the text stack and neither shows on a fixture. Our lock has
  swash 0.2.10 where both fulgur binaries have 0.2.7: that one exists today, with
  the pinned 0.40.0. And 0.41.0's lock moves ICU4X from 2.2.0 to 2.3.0, which our
  lock would follow only if we raised it too.

