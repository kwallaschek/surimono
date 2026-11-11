# The printer matrix: where each result comes from

`index.md` and `printers.json` are written by `fixtures/tools/printers.mjs`. This file
is hand-written. The tool relabels paths itself, so no machine path is committed.
There is no image in this folder because no cell differs between the two printers.

**This run.** 11 Nov 2026, on the fleet's Linux machine (Ubuntu 24.04, 2 cores,
poppler 24.02.0), against the committed references of Chromium 153.0.8010.12.

- **Printers.** fulgur-cli 0.40.0 (our pin) and 0.41.0 (the newest on crates.io,
  published 1 Nov 2026), each from crates.io with
  `cargo install fulgur-cli --version =<version> --locked --root <its own root>`
  and rustc 1.97.0, the stable toolchain `scripts/ci/nightly-extra.sh` installs
  0.40.0 with. `printers.json` has each binary's sha256.
- **surimono.** The CLI and `surimono-page-check` in release mode from main at
  90427b9e, default limits. Both printers were given the same bytes for every page
  (`surimono_outputs_sha256`).
- **Command.** `printers.mjs --runs 5 --printer 0.40.0=… --printer 0.41.0=…`, started
  while holding the fleet's build lock, so no build ran beside the timed prints.
  Other agents' tests could.
- `de_DE.UTF-8` isn't installed here, as in `fixtures/checkpoint-3`.

**Result.** fulgur 0.41.0 prints all 35 cells pixel for pixel as 0.40.0 does: the
6 real-world pages and their 6 floors, the 11 island fixture pages and the 12
islands alone. Every verdict and every score is the same. 2 of 6 real-world pages
pass with either version.

The 12 island PDFs are byte for byte the same. The 23 page PDFs are 78 or 79 bytes
longer with 0.41.0, which now writes the document's `<html lang>` into the PDF
(`/Lang` in the catalog and `dc:language` in the XMP). Checked on the 11 island
fixture pages and, printed by hand, on `alpine-invoice` and `marked-report`:
nothing else differs, and `pdftotext` and `pdffonts` give the same output for both
versions.

**Time and memory.** One `fulgur render` process per page, 23 pages and floors,
medians of 5 runs each.

| | fulgur 0.40.0 | fulgur 0.41.0 |
|---|---|---|
| Print time, median over the pages | 33.7 ms | 29.7 ms |
| Print time, sum over the pages | 782 ms | 724 ms |
| Peak PSS, median over the pages | 43.4 MB | 44.9 MB |
| Peak PSS, largest (`marked-report`) | 50.9 MB | 51.1 MB |
| Kernel's peak RSS, median over the pages | 45.9 MB | 47.1 MB |

Neither version is measurably faster. The matrix was run six times on this machine,
and the committed run is the first with the right commit in it. Its 4 ms are the
order of the run, not the printer: 0.40.0's pages are printed first, and with the
order swapped the medians were 29.2 ms for 0.40.0 and 28.4 ms for 0.41.0. Three
runs before the machine restarted gave 34 to 36 ms for 0.40.0 and 34 to 38 ms for
0.41.0. Per page, the committed run has 0.41.0 between 0.75 and 1.06 times 0.40.0.

0.41.0 uses up to 1.5 MB more. In the three runs after the restart its median peak
PSS was 44.9 MB each time against 43.4 or 43.5 MB, whichever printed first; in the
three before, 43.9 to 44.1 MB against 43.7 to 43.9 MB. The sampler reads every
millisecond here and saw each print 14 to 45 times.

**The gaps of `checkpoint-3/NOTES.md`.** 0.41.0 closes none and opens none. Each
failing page is the same pixels as before, on surimono's output and on Chrome's
own DOM.

| Page | Score (shape), both versions | Gap | With 0.41.0 |
|---|---|---|---|
| `d3-charts` | 100% | A block `<svg>` ignores its `width` and `height` attributes | unchanged |
| `katex-autorender` | 85.2% | `@font-face` URLs aren't loaded; `vertical-align`, negative margins and inline-block struts; `E = mc²` lost | unchanged |
| `marked-report` | 67.5% | List markers against the text, no synthesized italic, inline `code` without its background, a `<pre>` one line taller | unchanged |
| `noscript-message` | 100% | `border-bottom` alone prints black | unchanged |

`vegalite-line-ja`'s island alone still fails the shape check at 10.2% with both
(`docs/COMPAT.md`). The gaps the island page template works around (`fixtures/README.md`,
"Baseline") aren't measured here, because the fixtures avoid them.

**Dependencies against our pins.** From each version's own `Cargo.lock`, which
`--locked` builds with, and the repository's `Cargo.lock` at 90427b9e.

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
| icu_segmenter under stylo, and the ICU4X crates with it | 2.2.0 | **2.3.0** | 2.2.0 |
| fulgur's own skrifa and read-fonts | 0.44.0, 0.41.0 | 0.47.0, 0.44.0 | not used |
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

**What isn't measured.** A `fulgur-raikiri` 0.41.0 crate was published with this
release ("Raikiri layout backend under development for fulgur"). It is a library:
`fulgur render --help` of the 0.41.0 CLI has no option that selects it, and
`fulgur-cli` has no feature for it, so the CLI prints with Blitz only and these
numbers say nothing about Raikiri.
