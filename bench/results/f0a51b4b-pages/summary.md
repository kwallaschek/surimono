# Page-set benchmark

The page path: each page's own scripts run, then the page is printed. Grover runs the scripts in Chrome and prints there; surimono runs them in QuickJS (`render_page`, printer-safe output on) and a browserless printer prints its HTML. The pages are the real-world fixture pages as they are (`fixtures/real-world`, their libraries pinned in `manifest.toml`) and long documents written by `bench/pages/gen-pages.mjs` (their sha256 in `bench/pages/SHA256SUMS`).

| Page | Kind | Title | Bytes | sha256 |
|---|---|---|---|---|
| alpine-invoice | real-world | 請求書 | 3,251 | `0bb09b1b1b85` |
| d3-charts | real-world | Regional sales | 4,233 | `4175a391cada` |
| highlightjs-code | real-world | Code samples | 2,555 | `2839f7411199` |
| katex-autorender | real-world | Formula sheet | 2,100 | `49ad8effc031` |
| marked-report | real-world | Quarterly engineering notes | 2,416 | `437b1b1c3ce1` |
| noscript-message | real-world | Delivery report | 1,969 | `2c00f89a4133` |
| long-report | generated | Annual operations report FY2026 | 1,473,087 | `a4d5fd73da0b` |
| long-table | generated | 総勘定元帳 FY2026 | 344,430 | `d698b7f2b4d7` |

Measured on a local machine: Intel(R) Xeon(R) Processor @ 2.10GHz, 2 cores, 7.8 GB, load average 2.28 at the start and 1.93 at the end. 20 timed runs per cell (cold: interleaved across systems, one process each; warm: one process), 5 separate memory runs for cold and one process for warm, page by page.

**Other work was running on this machine during the benchmark (see the load average); treat the times as indicative. The CI run is the reference.**

## Results

Time per document: median (p90). Peak memory: PSS summed over the whole process tree (Chrome's helpers, and the Raikiri process, included), median of the memory runs; summed RSS in brackets. Quality: the system's cold PDF against the page's committed Chrome reference (real-world pages only), pass or **FAIL** and the shape score, by the pass rule of `fixtures/README.md`.

| Page | System | Cold | Warm | Peak memory, cold | Peak memory, warm | Pages printed | Quality | Status |
|---|---|---|---|---|---|---|---|---|
| alpine-invoice | Grover (Chrome) | 1,362 ms (1,725 ms) | 732 ms (770 ms) | 470 MB (1,072 MB) | 488 MB (1,116 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| alpine-invoice | surimono + fulgur (Ruby) | 312 ms (380 ms) | 42 ms (45 ms) | 104 MB (106 MB) | 109 MB (111 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| alpine-invoice | surimono + Raikiri (indicative) | 318 ms (400 ms) | 66 ms (78 ms) | 112 MB (116 MB) | 115 MB (119 MB) | cold 1 / warm 1 | pass · 0.0% | measured; **indicative system** |
| alpine-invoice | Grover (chrome-headless-shell) | 905 ms (994 ms) | 490 ms (523 ms) | 306 MB (594 MB) | 311 MB (620 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| d3-charts | Grover (Chrome) | 1,078 ms (1,169 ms) | 467 ms (515 ms) | 422 MB (980 MB) | 442 MB (1,075 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| d3-charts | surimono + fulgur (Ruby) | 353 ms (390 ms) | 76 ms (86 ms) | 99 MB (101 MB) | 105 MB (107 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| d3-charts | surimono + Raikiri (indicative) | 355 ms (406 ms) | 100 ms (114 ms) | 115 MB (119 MB) | 120 MB (124 MB) | cold 1 / warm 1 | pass · 0.0% | measured; **indicative system** |
| d3-charts | Grover (chrome-headless-shell) | 754 ms (821 ms) | 347 ms (382 ms) | 288 MB (575 MB) | 280 MB (589 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| highlightjs-code | Grover (Chrome) | 1,079 ms (1,210 ms) | 525 ms (573 ms) | 430 MB (988 MB) | 437 MB (1,068 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| highlightjs-code | surimono + fulgur (Ruby) | 317 ms (390 ms) | 76 ms (85 ms) | 98 MB (100 MB) | 100 MB (102 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| highlightjs-code | surimono + Raikiri (indicative) | 332 ms (452 ms) | 103 ms (115 ms) | 114 MB (117 MB) | 116 MB (120 MB) | cold 1 / warm 1 | pass · 0.0% | measured; **indicative system** |
| highlightjs-code | Grover (chrome-headless-shell) | 750 ms (822 ms) | 410 ms (453 ms) | 291 MB (580 MB) | 286 MB (597 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| katex-autorender | Grover (Chrome) | 1,115 ms (1,186 ms) | 523 ms (583 ms) | 434 MB (992 MB) | 446 MB (1,076 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| katex-autorender | surimono + fulgur (Ruby) | 367 ms (404 ms) | 92 ms (100 ms) | 110 MB (112 MB) | 119 MB (121 MB) | cold 1 / warm 1 | **FAIL** · 85.2% | measured |
| katex-autorender | surimono + Raikiri (indicative) | 494 ms (547 ms) | 261 ms (275 ms) | 120 MB (124 MB) | 125 MB (129 MB) | cold 1 / warm 1 | pass · 0.0% | measured; **indicative system** |
| katex-autorender | Grover (chrome-headless-shell) | 786 ms (835 ms) | 403 ms (421 ms) | 297 MB (586 MB) | 298 MB (608 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| marked-report | Grover (Chrome) | 1,264 ms (1,339 ms) | 693 ms (751 ms) | 476 MB (1,086 MB) | 494 MB (1,126 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| marked-report | surimono + fulgur (Ruby) | 291 ms (345 ms) | 45 ms (51 ms) | 109 MB (111 MB) | 118 MB (120 MB) | cold 1 / warm 1 | **FAIL** · 67.5% | measured |
| marked-report | surimono + Raikiri (indicative) | 299 ms (324 ms) | 66 ms (82 ms) | 111 MB (115 MB) | 115 MB (119 MB) | cold 1 / warm 1 | **FAIL** · 72.1% | measured; **indicative system** |
| marked-report | Grover (chrome-headless-shell) | 883 ms (950 ms) | 513 ms (568 ms) | 308 MB (598 MB) | 316 MB (629 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| noscript-message | Grover (Chrome) | 1,003 ms (1,089 ms) | 396 ms (433 ms) | 417 MB (1,010 MB) | 418 MB (1,046 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| noscript-message | surimono + fulgur (Ruby) | 286 ms (324 ms) | 24 ms (26 ms) | 93 MB (95 MB) | 100 MB (102 MB) | cold 1 / warm 1 | pass · 9.7% | measured |
| noscript-message | surimono + Raikiri (indicative) | 300 ms (332 ms) | 49 ms (61 ms) | 98 MB (102 MB) | 115 MB (119 MB) | cold 1 / warm 1 | pass · 0.0% | measured; **indicative system** |
| noscript-message | Grover (chrome-headless-shell) | 693 ms (740 ms) | 299 ms (389 ms) | 278 MB (562 MB) | 269 MB (576 MB) | cold 1 / warm 1 | pass · 0.0% | measured |
| long-report | Grover (Chrome) | 3,712 ms (4,008 ms) | 3,085 ms (3,307 ms) | 874 MB (1,426 MB) | 1,152 MB (1,731 MB) | cold 402 / warm 402 | — | measured |
| long-report | surimono + fulgur (Ruby) | 2,014 ms (2,094 ms) | 1,686 ms (1,735 ms) | 282 MB (284 MB) | 360 MB (362 MB) | cold 404 / warm 404 | — | measured |
| long-report | surimono + Raikiri (indicative) | 3,626 ms (3,824 ms) | 3,448 ms (3,595 ms) | 1,076 MB (1,080 MB) | 1,135 MB (1,139 MB) | cold 400 / warm 400 | — | measured; **indicative system** |
| long-report | Grover (chrome-headless-shell) | 2,991 ms (3,123 ms) | 2,565 ms (2,730 ms) | 480 MB (767 MB) | 627 MB (937 MB) | cold 402 / warm 402 | — | measured |
| long-table | Grover (Chrome) | 3,370 ms (3,538 ms) | 2,900 ms (3,058 ms) | 918 MB (1,478 MB) | 1,140 MB (1,728 MB) | cold 49 / warm 49 | — | measured |
| long-table | surimono + fulgur (Ruby) | 1,017 ms (1,072 ms) | 761 ms (868 ms) | 240 MB (242 MB) | 283 MB (285 MB) | cold 47 / warm 47 | — | measured |
| long-table | surimono + Raikiri (indicative) | 3,068 ms (3,241 ms) | 2,883 ms (3,139 ms) | 583 MB (587 MB) | 632 MB (636 MB) | cold 49 / warm 49 | — | measured; **indicative system** |
| long-table | Grover (chrome-headless-shell) | 2,432 ms (2,638 ms) | 2,056 ms (2,305 ms) | 612 MB (903 MB) | 770 MB (1,085 MB) | cold 49 / warm 49 | — | measured |

## Against Grover

Ratios are Grover ÷ the other system, per page: above 1× the other system is faster or uses less memory. A ratio is shown only when both systems finished every run on that page, and it is marked indicative with fewer than 20 timed runs or 5 memory runs, and always for surimono + Raikiri. `docs/SPEC.md`'s targets (2× time, 5× peak PSS) are set on the islands reference report; the tallies below count against them for orientation, not as a verdict on a target.

| Page | Comparison | Cold time | Warm time | Peak memory, cold: PSS (RSS) | Peak memory, warm: PSS (RSS) |
|---|---|---|---|---|---|
| alpine-invoice | Grover (Chrome) ÷ surimono + fulgur (Ruby) | 4.4× | 17.5× | 4.5× (10.1×) | 4.5× (10.1×) |
| alpine-invoice | Grover (Chrome) ÷ surimono + Raikiri (indicative) | 4.3× indicative | 11.0× indicative | 4.2× (9.2×) indicative | 4.3× (9.4×) indicative |
| alpine-invoice | Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | 2.9× | 11.7× | 2.9× (5.6×) | 2.9× (5.6×) |
| alpine-invoice | Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | 2.8× indicative | 7.4× indicative | 2.7× (5.1×) indicative | 2.7× (5.2×) indicative |
| d3-charts | Grover (Chrome) ÷ surimono + fulgur (Ruby) | 3.1× | 6.1× | 4.3× (9.7×) | 4.2× (10.0×) |
| d3-charts | Grover (Chrome) ÷ surimono + Raikiri (indicative) | 3.0× indicative | 4.7× indicative | 3.7× (8.2×) indicative | 3.7× (8.7×) indicative |
| d3-charts | Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | 2.1× | 4.5× | 2.9× (5.7×) | 2.7× (5.5×) |
| d3-charts | Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | 2.1× indicative | 3.5× indicative | 2.5× (4.8×) indicative | 2.3× (4.8×) indicative |
| highlightjs-code | Grover (Chrome) ÷ surimono + fulgur (Ruby) | 3.4× | 6.9× | 4.4× (9.9×) | 4.4× (10.4×) |
| highlightjs-code | Grover (Chrome) ÷ surimono + Raikiri (indicative) | 3.2× indicative | 5.1× indicative | 3.8× (8.4×) indicative | 3.8× (8.9×) indicative |
| highlightjs-code | Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | 2.4× | 5.4× | 3.0× (5.8×) | 2.9× (5.8×) |
| highlightjs-code | Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | 2.3× indicative | 4.0× indicative | 2.6× (4.9×) indicative | 2.5× (5.0×) indicative |
| katex-autorender | Grover (Chrome) ÷ surimono + fulgur (Ruby) | 3.0× | 5.7× | 4.0× (8.9×) | 3.8× (8.9×) |
| katex-autorender | Grover (Chrome) ÷ surimono + Raikiri (indicative) | 2.3× indicative | 2.0× indicative | 3.6× (8.0×) indicative | 3.6× (8.4×) indicative |
| katex-autorender | Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | 2.1× | 4.4× | 2.7× (5.2×) | 2.5× (5.0×) |
| katex-autorender | Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | 1.6× indicative | 1.5× indicative | 2.5× (4.7×) indicative | 2.4× (4.7×) indicative |
| marked-report | Grover (Chrome) ÷ surimono + fulgur (Ruby) | 4.3× | 15.4× | 4.4× (9.8×) | 4.2× (9.3×) |
| marked-report | Grover (Chrome) ÷ surimono + Raikiri (indicative) | 4.2× indicative | 10.5× indicative | 4.3× (9.5×) indicative | 4.3× (9.5×) indicative |
| marked-report | Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | 3.0× | 11.4× | 2.8× (5.4×) | 2.7× (5.2×) |
| marked-report | Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | 3.0× indicative | 7.7× indicative | 2.8× (5.2×) indicative | 2.7× (5.3×) indicative |
| noscript-message | Grover (Chrome) ÷ surimono + fulgur (Ruby) | 3.5× | 16.5× | 4.5× (10.7×) | 4.2× (10.3×) |
| noscript-message | Grover (Chrome) ÷ surimono + Raikiri (indicative) | 3.3× indicative | 8.1× indicative | 4.2× (9.9×) indicative | 3.6× (8.8×) indicative |
| noscript-message | Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | 2.4× | 12.5× | 3.0× (5.9×) | 2.7× (5.7×) |
| noscript-message | Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | 2.3× indicative | 6.1× indicative | 2.8× (5.5×) indicative | 2.3× (4.8×) indicative |
| long-report | Grover (Chrome) ÷ surimono + fulgur (Ruby) | 1.8× | 1.8× | 3.1× (5.0×) | 3.2× (4.8×) |
| long-report | Grover (Chrome) ÷ surimono + Raikiri (indicative) | 1.0× indicative | 0.9× (slower) indicative | 0.8× (1.3×) (more memory) indicative | 1.0× (1.5×) indicative |
| long-report | Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | 1.5× | 1.5× | 1.7× (2.7×) | 1.7× (2.6×) |
| long-report | Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | 0.8× (slower) indicative | 0.7× (slower) indicative | 0.4× (0.7×) (more memory) indicative | 0.6× (0.8×) (more memory) indicative |
| long-table | Grover (Chrome) ÷ surimono + fulgur (Ruby) | 3.3× | 3.8× | 3.8× (6.1×) | 4.0× (6.1×) |
| long-table | Grover (Chrome) ÷ surimono + Raikiri (indicative) | 1.1× indicative | 1.0× indicative | 1.6× (2.5×) indicative | 1.8× (2.7×) indicative |
| long-table | Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | 2.4× | 2.7× | 2.5× (3.7×) | 2.7× (3.8×) |
| long-table | Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | 0.8× (slower) indicative | 0.7× (slower) indicative | 1.0× (1.5×) indicative | 1.2× (1.7×) indicative |

### Tallies

Pages counted: those where both systems are fully measured with the runs above (for surimono + Raikiri, every page where both finished, all indicative). Range: the lowest and highest ratio over those pages.

| Comparison | | Pages | Faster / less memory (> 1×) | At least the SPEC's ratio | Range |
|---|---|---|---|---|---|
| Grover (Chrome) ÷ surimono + fulgur (Ruby) | cold time | 8 | 8 of 8 | 7 of 8 (≥ 2×) | 1.8× to 4.4× |
| Grover (Chrome) ÷ surimono + fulgur (Ruby) | warm time | 8 | 8 of 8 | 7 of 8 (≥ 2×) | 1.8× to 17.5× |
| Grover (Chrome) ÷ surimono + fulgur (Ruby) | peak PSS, cold | 8 | 8 of 8 | 0 of 8 (≥ 5×) | 3.1× to 4.5× |
| Grover (Chrome) ÷ surimono + fulgur (Ruby) | peak PSS, warm | 8 | 8 of 8 | 0 of 8 (≥ 5×) | 3.2× to 4.5× |
| Grover (Chrome) ÷ surimono + Raikiri (indicative) | cold time | 8 | 8 of 8 | 6 of 8 (≥ 2×) | 1.0× to 4.3× |
| Grover (Chrome) ÷ surimono + Raikiri (indicative) | warm time | 8 | 7 of 8 | 6 of 8 (≥ 2×) | 0.9× to 11.0× |
| Grover (Chrome) ÷ surimono + Raikiri (indicative) | peak PSS, cold | 8 | 7 of 8 | 0 of 8 (≥ 5×) | 0.8× to 4.3× |
| Grover (Chrome) ÷ surimono + Raikiri (indicative) | peak PSS, warm | 8 | 8 of 8 | 0 of 8 (≥ 5×) | 1.0× to 4.3× |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | cold time | 8 | 8 of 8 | 7 of 8 (≥ 2×) | 1.5× to 3.0× |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | warm time | 8 | 8 of 8 | 7 of 8 (≥ 2×) | 1.5× to 12.5× |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | peak PSS, cold | 8 | 8 of 8 | 0 of 8 (≥ 5×) | 1.7× to 3.0× |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | peak PSS, warm | 8 | 8 of 8 | 0 of 8 (≥ 5×) | 1.7× to 2.9× |
| Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | cold time | 8 | 6 of 8 | 5 of 8 (≥ 2×) | 0.8× to 3.0× |
| Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | warm time | 8 | 6 of 8 | 5 of 8 (≥ 2×) | 0.7× to 7.7× |
| Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | peak PSS, cold | 8 | 7 of 8 | 0 of 8 (≥ 5×) | 0.4× to 2.8× |
| Grover (chrome-headless-shell) ÷ surimono + Raikiri (indicative) | peak PSS, warm | 8 | 7 of 8 | 0 of 8 (≥ 5×) | 0.6× to 2.7× |

### Print quality against Chrome

| System | Real-world pages that pass | Not scored (no PDF, or not reached) |
|---|---|---|
| Grover (Chrome) | 6 of 6 | 0 |
| Grover (chrome-headless-shell) | 6 of 6 | 0 |
| surimono + fulgur (Ruby) | 4 of 6 | 0 |
| surimono + Raikiri (indicative) | 5 of 6 | 0 |

## Details

| Page | System | Cold: setup / render inside the process (median) | Cold CPU time (median) | Warm phases (median) | Page report (surimono) | Processes at peak |
|---|---|---|---|---|---|---|
| alpine-invoice | Grover (Chrome) | 45 ms / 1,135 ms | 1,665 ms | grover 732 ms | — | 14 |
| alpine-invoice | surimono + fulgur (Ruby) | 25 ms / 91 ms | 295 ms | page 30 ms, pdf 11 ms | settled idle; uncaught 0; resources failed 0 | 1 |
| alpine-invoice | surimono + Raikiri (indicative) | 17 ms / 107 ms | 305 ms | page 33 ms, pdf 33 ms | settled idle; uncaught 0; resources failed 0 | 2 |
| alpine-invoice | Grover (chrome-headless-shell) | 45 ms / 678 ms | 1,030 ms | grover 490 ms | — | 9 |
| d3-charts | Grover (Chrome) | 45 ms / 832 ms | 1,405 ms | grover 467 ms | — | 13 |
| d3-charts | surimono + fulgur (Ruby) | 26 ms / 135 ms | 350 ms | page 57 ms, pdf 19 ms | settled idle; uncaught 0; resources failed 0 | 1 |
| d3-charts | surimono + Raikiri (indicative) | 17 ms / 146 ms | 345 ms | page 63 ms, pdf 36 ms | settled idle; uncaught 0; resources failed 0 | 2 |
| d3-charts | Grover (chrome-headless-shell) | 44 ms / 519 ms | 910 ms | grover 347 ms | — | 9 |
| highlightjs-code | Grover (Chrome) | 45 ms / 854 ms | 1,440 ms | grover 525 ms | — | 13 |
| highlightjs-code | surimono + fulgur (Ruby) | 24 ms / 107 ms | 320 ms | page 62 ms, pdf 11 ms | settled idle; uncaught 0; resources failed 0 | 1 |
| highlightjs-code | surimono + Raikiri (indicative) | 17 ms / 141 ms | 330 ms | page 66 ms, pdf 37 ms | settled idle; uncaught 0; resources failed 0 | 2 |
| highlightjs-code | Grover (chrome-headless-shell) | 46 ms / 519 ms | 905 ms | grover 410 ms | — | 9 |
| katex-autorender | Grover (Chrome) | 44 ms / 892 ms | 1,490 ms | grover 523 ms | — | 13 |
| katex-autorender | surimono + fulgur (Ruby) | 34 ms / 141 ms | 365 ms | page 67 ms, pdf 22 ms | settled idle; uncaught 0; resources failed 0 | 1 |
| katex-autorender | surimono + Raikiri (indicative) | 17 ms / 299 ms | 485 ms | page 70 ms, pdf 192 ms | settled idle; uncaught 0; resources failed 0 | 2 |
| katex-autorender | Grover (chrome-headless-shell) | 44 ms / 563 ms | 945 ms | grover 403 ms | — | 9 |
| marked-report | Grover (Chrome) | 44 ms / 1,043 ms | 1,640 ms | grover 693 ms | — | 14 |
| marked-report | surimono + fulgur (Ruby) | 24 ms / 91 ms | 290 ms | page 28 ms, pdf 14 ms | settled idle; uncaught 0; resources failed 0 | 1 |
| marked-report | surimono + Raikiri (indicative) | 17 ms / 99 ms | 295 ms | page 31 ms, pdf 34 ms | settled idle; uncaught 0; resources failed 0 | 2 |
| marked-report | Grover (chrome-headless-shell) | 43 ms / 659 ms | 1,035 ms | grover 513 ms | — | 9 |
| noscript-message | Grover (Chrome) | 45 ms / 776 ms | 1,330 ms | grover 397 ms | — | 13 |
| noscript-message | surimono + fulgur (Ruby) | 25 ms / 78 ms | 285 ms | page 16 ms, pdf 7 ms | settled idle; uncaught 0; resources failed 0 | 1 |
| noscript-message | surimono + Raikiri (indicative) | 16 ms / 90 ms | 300 ms | page 19 ms, pdf 30 ms | settled idle; uncaught 0; resources failed 0 | 2 |
| noscript-message | Grover (chrome-headless-shell) | 44 ms / 455 ms | 815 ms | grover 299 ms | — | 9 |
| long-report | Grover (Chrome) | 44 ms / 3,462 ms | 4,300 ms | grover 3,085 ms | — | 14 |
| long-report | surimono + fulgur (Ruby) | 28 ms / 1,767 ms | 2,000 ms | page 104 ms, pdf 1,584 ms | settled idle; uncaught 0; resources failed 0 | 1 |
| long-report | surimono + Raikiri (indicative) | 19 ms / 3,417 ms | 4,350 ms | page 112 ms, pdf 3,342 ms | settled idle; uncaught 0; resources failed 0 | 2 |
| long-report | Grover (chrome-headless-shell) | 46 ms / 2,756 ms | 3,275 ms | grover 2,565 ms | — | 9 |
| long-table | Grover (Chrome) | 44 ms / 3,131 ms | 4,050 ms | grover 2,899 ms | — | 14 |
| long-table | surimono + fulgur (Ruby) | 28 ms / 794 ms | 1,010 ms | page 237 ms, pdf 494 ms | settled idle; uncaught 0; resources failed 0 | 1 |
| long-table | surimono + Raikiri (indicative) | 20 ms / 2,847 ms | 3,210 ms | page 253 ms, pdf 2,645 ms | settled idle; uncaught 0; resources failed 0 | 2 |
| long-table | Grover (chrome-headless-shell) | 44 ms / 2,210 ms | 2,695 ms | grover 2,056 ms | — | 9 |

Left out of Grover (Chrome)'s warm peak on alpine-invoice: the Node process that only keeps Chrome open, peak 68 MB PSS (70 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak on alpine-invoice: the Node process that only keeps Chrome open, peak 69 MB PSS (71 MB RSS). Its Chrome processes are counted.

Left out of Grover (Chrome)'s warm peak on d3-charts: the Node process that only keeps Chrome open, peak 68 MB PSS (70 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak on d3-charts: the Node process that only keeps Chrome open, peak 69 MB PSS (71 MB RSS). Its Chrome processes are counted.

Left out of Grover (Chrome)'s warm peak on highlightjs-code: the Node process that only keeps Chrome open, peak 67 MB PSS (70 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak on highlightjs-code: the Node process that only keeps Chrome open, peak 68 MB PSS (70 MB RSS). Its Chrome processes are counted.

Left out of Grover (Chrome)'s warm peak on katex-autorender: the Node process that only keeps Chrome open, peak 68 MB PSS (70 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak on katex-autorender: the Node process that only keeps Chrome open, peak 68 MB PSS (71 MB RSS). Its Chrome processes are counted.

Left out of Grover (Chrome)'s warm peak on marked-report: the Node process that only keeps Chrome open, peak 68 MB PSS (70 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak on marked-report: the Node process that only keeps Chrome open, peak 68 MB PSS (70 MB RSS). Its Chrome processes are counted.

Left out of Grover (Chrome)'s warm peak on noscript-message: the Node process that only keeps Chrome open, peak 69 MB PSS (72 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak on noscript-message: the Node process that only keeps Chrome open, peak 69 MB PSS (71 MB RSS). Its Chrome processes are counted.

Left out of Grover (Chrome)'s warm peak on long-report: the Node process that only keeps Chrome open, peak 68 MB PSS (70 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak on long-report: the Node process that only keeps Chrome open, peak 72 MB PSS (75 MB RSS). Its Chrome processes are counted.

Left out of Grover (Chrome)'s warm peak on long-table: the Node process that only keeps Chrome open, peak 68 MB PSS (71 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak on long-table: the Node process that only keeps Chrome open, peak 68 MB PSS (70 MB RSS). Its Chrome processes are counted.

## Versions

| | |
|---|---|
| Chromium (Grover) | Google Chrome for Testing 153.0.8010.12 |
| chrome-headless-shell (Grover, headless shell) | Google Chrome for Testing 153.0.8010.12 |
| grover gem | 1.2.10 |
| puppeteer | 25.11.0 |
| fulgur gem | 0.40.0 |
| Raikiri printer | fulgur-raikiri 0.41.0 (fixtures/tools/raikiri-print, sha256 90c463fa1601cb1fbb590fc0884eb76c0f4e9828f7cc01516c9c1e16ef399a52) |
| surimono gem | 0.1.0 (native 0.1.0) |
| surimono commit | f0a51b4bac1c20d6166a381d2c236d67662514c9 |
| Ruby | ruby 3.3.6 (2024-11-05 revision 75015d4c1f) [x86_64-linux] |
| Node | v22.23.4 |

## Method

- **Grover (Chrome)**: Grover 1.2.10, Puppeteer, Chrome; the page's own scripts run in Chrome, which prints once they are done by the Chrome references' rule (two animation frames and a task after load). Warm: Ruby process and Chrome reused (browser_ws_endpoint); the Node process that only keeps Chrome open is left out of the memory sum (reported separately).
- **surimono + fulgur (Ruby)**: Surimono::Renderer#render_page (the page's own scripts in QuickJS on the virtual clock, printer-safe output on, the page's <script src> and <link href> files given as resources:), then the fulgur gem 0.40.0 prints the HTML, in one Ruby process.
- **surimono + Raikiri (indicative)**: As surimono + fulgur (Ruby), but fulgur's Raikiri backend (fulgur-raikiri 0.41.0 through fixtures/tools/raikiri-print, the printer matrix's driver) prints the HTML in a process of its own, started by the Ruby process for every document: Raikiri has no Ruby binding, so its warm row reuses the renderer but never a warm printer. Indicative: not the setup a user of a gem would run.
- **Grover (chrome-headless-shell)**: As Grover (Chrome), on chrome-headless-shell of the same build (Puppeteer's headless: "shell"), the leanest Chrome a Grover user can run.
- **Cold** is the wall time of a new process from spawn to exit: Ruby boot and Bundler, setup, one render, the PDF written. **Warm** is one render inside a long-lived process after 2 untimed renders, timed by the process. Each page is measured completely (all systems, all phases) before the next.
- **Grover** prints a file:// URL with `wait_until: load`, then waits until the page's scripts are done by the rule the Chrome references use (two animation frames and a task after load), its fastest correct setting; Grover's default for URLs, `networkidle2`, waits 500 ms after the last request. `--no-sandbox`, as in the islands benchmark.
- **surimono** gets the page's `<script src>` and `<link href>` files as `resources:` (no network, no files otherwise), as a Rails app passes its assets, and its default limits.
- **Memory** is sampled from `/proc/<pid>/smaps_rollup` for every process in the tree, aiming at one sample every 10 ms (achieved: 10 to 36 ms on average per run); a peak shorter than the interval can be missed. A process that exists but can't be read marks the run's memory incomplete instead of counting as zero.
- **Fonts**: every process gets a fontconfig file that only knows `fixtures/fonts`; the printers get the same files as fonts.
- **No output**: a system that printed nothing on a page says so; how a printer fails stays out of these files.
- **surimono's runtime pool**: 2 runtimes (default (CPUs clamped to 1..8), 2 CPUs here).

