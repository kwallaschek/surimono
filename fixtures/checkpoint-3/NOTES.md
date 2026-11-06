# Real-world pages printed: where each result comes from

`index.md`, `print-pages.json` and the per-page folders are written by
`fixtures/tools/print-pages.mjs`, and `hostile-page-results.json` by
`surimono-page-check hostile`. This file is hand-written. Paths are relabelled after
the run, so no machine path is committed. PNGs are reduced to 256 colours.

**This run.** `surimono-page-check` in release mode from main at e879ec95 (after #149 to #161), default
limits, fulgur-cli 0.40.0 from crates.io, Chromium 153.0.8010.12's committed
references, on the fleet's Linux machine (poppler 24.02.0). `de_DE.UTF-8` isn't
installed here, so the second process sets the variables and doesn't change libc's
locale, as in `fixtures/checkpoint-2`; `Asia/Tokyo` is in effect.

**Result.** 2 of 6 pages pass; `alpine-invoice` and `highlightjs-code`. All 6 are
byte-identical, twice in one process and twice in a fresh one. **Every page prints
pixel for pixel like its floor**, fulgur printing Chrome's own DOM, so all four
failures are fulgur's and surimono's output is what Chrome's scripts left.

| Page | Score (shape) | What fulgur 0.40.0 does | What it needs |
|---|---|---|---|
| `d3-charts` | 100% | An `<svg>` with `display: block` ignores its `width` and `height` attributes and fills the content box. A CSS width works | Upstream, or `svg { width: … }` in the fixture |
| `katex-autorender` | 85.2% | `@font-face` URLs aren't loaded, so the fonts come with `--font`. KaTeX's `vertical-align`, negative margins and inline-block struts don't lay out as in Chrome, and `E = mc²` is lost | Upstream layout. No workaround in a fixture |
| `marked-report` | 67.5% | List markers sit against the text, `<em>` has no synthesized italic, inline `code` loses its background, and a `<pre>` is one line taller | Upstream, or marked's output kept to what fulgur prints |
| `noscript-message` | 100% | `border-bottom` alone prints black, whatever its colour, on a block or a flex row; `border` on all sides keeps its colour. Neither `<noscript>` message reaches the PDF | Upstream, or the fixture's rows without `border-bottom` |

**Hostile scripts.** 79 of 79 cases and steps end as the manifest says, through
`render_page`: no `Crash`, no `Hang`.
