# The printer matrix: where each result comes from

`index.md` and `printers.json` are written by `fixtures/tools/printers.mjs`, and
`reproducers/reproducers.md`, `reproducers.json` and the images beside them by
`fixtures/tools/reproducers.mjs`. This file is hand-written. The tools relabel paths
themselves, so no machine path is committed. The PNGs are reduced to 256 colours
(`convert -dither None -colors 256`); nothing else is touched after a run.

**This run.** 12 Nov 2026, on the fleet's Linux machine (Ubuntu 24.04, 2 cores,
poppler 24.02.0), against the committed references of Chromium 153.0.8010.12.

- **fulgur 0.40.0 and 0.41.0.** fulgur-cli 0.40.0 (our pin) and 0.41.0 (the newest
  on crates.io, published 1 Nov 2026), each from crates.io with
  `cargo install fulgur-cli --version =<version> --locked --root <its own root>`
  and rustc 1.97.0. MIT OR Apache-2.0, https://github.com/fulgur-rs/fulgur.
- **Raikiri.** `fixtures/tools/raikiri-print`, in release mode with its committed
  `Cargo.lock` and rustc 1.89.0, on the library crate `fulgur-raikiri` 0.41.0 from
  crates.io (MIT OR Apache-2.0, the same repository, published with fulgur 0.41.0 as
  "Raikiri layout backend under development for fulgur"). The published fulgur CLI
  has no option that selects Raikiri, which is why there is a driver.
- **surimono.** The CLI and `surimono-page-check` in release mode from main at
  9d363d0d, default limits. Every printer was given the same bytes for every page
  (`surimono_outputs_sha256`).
- **Command.** `printers.mjs --runs 5 --printer 0.40.0=… --printer 0.41.0=…
  --printer raikiri=…`, started while holding the fleet's build lock, so no build
  ran beside the timed prints. Other agents' tests could. `reproducers.mjs` with
  the same three printers, not under the lock: it times nothing.
- `printers.json` and `reproducers.json` have each binary's sha256.
- `de_DE.UTF-8` isn't installed here, as in `fixtures/checkpoint-3`.
- No printer failed to print a page: every cell has a score.

**Result.**

| | fulgur 0.40.0 | fulgur 0.41.0 | Raikiri (fulgur-raikiri 0.41.0) |
|---|---|---|---|
| Real-world pages that pass, of 6 | 2 | 2 | 5 |
| Their floors (Chrome's own DOM) that pass, of 6 | 2 | 2 | 5 |
| Island fixture pages that pass, of 11 | 11 | 11 | 6 |
| Islands alone that pass, of 12 | 11 | 11 | 11 |
| Reproducers that pass, of 13 | 2 | 2 | 8 |

- **fulgur 0.41.0** prints all 35 cells pixel for pixel as 0.40.0 does, and all 13
  reproducers with the same scores. The 12 island PDFs are byte for byte the same;
  the 23 page PDFs are 78 or 79 bytes longer, because 0.41.0 writes the document's
  `<html lang>` into the PDF.
- **Raikiri** passes three real-world pages that fulgur fails, `d3-charts`,
  `katex-autorender` and `noscript-message`, each at 0.0%, on surimono's output and
  on Chrome's own DOM alike. `marked-report` fails with it too, at 72.1% (67.5% with
  fulgur).
- **Raikiri fails five island fixture pages that fulgur passes**, the five with
  Japanese text: `echarts-bar-simple-ja` 25.7%, `echarts-mixed-dense-ja` 22.0%,
  `echarts-pie-ja` 61.8%, `vegalite-layered-dense-ja` 52.6% and `vegalite-line-ja`
  34.6% (0.0% to 7.3% with fulgur). The six Latin pages score as with fulgur. The
  cause is R1 below.
- The 12 islands alone are the same pixels with all three printers: an island is an
  SVG of outlines, and nothing in it is laid out as text.

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
| R1 space between Japanese and Latin letters or digits (new with Raikiri) | pass 0.4% | FAIL 49.2%; pass 0.4% with `text-autospace: no-autospace` on the paragraph | the five Japanese island pages fail, 22.0% to 61.8% | opened: Raikiri puts space between "2026" and "年" as CSS Text 4's initial `text-autospace: normal` says, Chromium 153 puts none; a page can switch it off |

Seven of the ten gaps are closed on their reproducer or their page, F2, F3 and half
of F4 are open, and R1 is new. F7 to F9 and B3, B5, B6 have no reproducer here.

**Time and memory.** One process per page, on the 23 pages and floors. The three
printers take turns on each page: a round prints it once with each, one round isn't
measured, then come 5 timed rounds and 5 sampled ones. Who starts moves on every
round and every page: of the 115 timed rounds 0.40.0 started 38, 0.41.0 39 and
Raikiri 38 (`print_order` in `printers.json`). Every figure here is in `index.md` or
follows from the single runs in `printers.json`.

| | fulgur 0.40.0 | fulgur 0.41.0 | Raikiri |
|---|---|---|---|
| Print time, median over the pages | 26.7 ms | 27.3 ms | 31.8 ms |
| Print time, sum of the pages' medians | 673 ms | 678 ms | 1114 ms |
| Peak PSS, median over the pages | 44.6 MB | 44.9 MB | 51.3 MB |
| Peak PSS, largest | 51.9 MB (`marked-report`) | 51.5 MB (`marked-report`) | 73.8 MB (`vegalite-scatter-dense-latin`) |

- **Raikiri, time.** Its median is higher than 0.40.0's on 17 of the 23 pages,
  lower on 5 and equal on 1; the difference per page runs from −2.0 ms to +159.6 ms
  and is +3.9 ms at the median page. On 11 pages every run of Raikiri is slower than
  every run of 0.40.0, on none is every run faster. Three pages carry most of the
  sum: `katex-autorender` takes 177 ms and its floor 206 ms where 0.40.0 takes 44
  and 47 ms, and `vegalite-scatter-dense-latin` 149 ms where 0.40.0 takes 63 ms.
- **Raikiri, memory.** Its median peak is higher on 21 pages and lower on 2
  (`marked-report` and its floor, by 1.6 and 1.7 MB); the difference per page runs
  from −1.7 MB to +25.9 MB and is +12.8 MB at the median page. On those 21 pages
  every sampled run of Raikiri is above every run of 0.40.0.
- **fulgur 0.41.0 against 0.40.0.** Time: lower on 14 pages, equal on 1, higher on
  8; per page −1.0 ms to +5.3 ms, −0.2 ms at the median page, and on no page does
  every run of one lie outside the other's runs. Memory: higher on 19 pages, lower
  on 4; per page −0.6 MB to +1.8 MB, +0.3 MB at the median page, and on 6 pages
  every run of 0.41.0 is above every run of 0.40.0.
- The sampler reads every millisecond here and saw each print 12 to 127 times.

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

