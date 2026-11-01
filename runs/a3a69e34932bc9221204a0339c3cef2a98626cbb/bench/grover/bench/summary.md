# Grover benchmark

Reference report: `bench/report/report.html` (6 pages, 6 chart islands: ECharts, Vega and Vega-Lite; Latin and Japanese; simple and dense). Fonts: Noto Sans and Noto Sans JP from `fixtures/fonts`.

The report is shaped around fulgur 0.40's layout gaps (`docs/COMPAT.md`): flex rows instead of `<table>`, fixed sizes instead of auto-sized flex items, one chart per page. That is right for comparing the same six pages in both engines, but it is not a typical Rails template; a page fulgur can't lay out yet isn't measured here.

Measured on GitHub Actions: AMD EPYC 7763 64-Core Processor, 4 cores, 15.6 GB, load average 1.86 at the start and 3.30 at the end. 20 timed runs per cell (cold: interleaved, one process each; warm: one process), 5 separate memory runs for cold and one process for warm.

The bench/rust release build took 2.6 min in this run (not part of any number below).

## Results

Time per document: median (p90). Peak memory: PSS summed over the whole process tree (Chrome's helpers included), median of the memory runs; summed RSS in brackets.

| System | Cold | Warm | Peak memory, cold | Peak memory, warm | Status |
|---|---|---|---|---|---|
| Grover (Chrome) | 2,184 ms (2,229 ms) | 1,645 ms (1,804 ms) | 587 MB (1,284 MB) | 719 MB (1,432 MB) | measured |
| surimono + fulgur (Ruby) | 932 ms (944 ms) | 719 ms (725 ms) | 98 MB (101 MB) | 127 MB (130 MB) | measured |
| surimono + fulgur (Rust) | 778 ms (786 ms) | 719 ms (734 ms) | 74 MB (76 MB) | 87 MB (90 MB) | measured |
| fulgur only, Chrome's SVGs (reference point) | 366 ms (369 ms) | 184 ms (186 ms) | 66 MB (69 MB) | 80 MB (83 MB) | measured; **reference point, not surimono** (no JavaScript runs; Chrome's SVG text is not outlined) |
| Grover (chrome-headless-shell) | 1,662 ms (1,681 ms) | 1,282 ms (1,314 ms) | 382 MB (724 MB) | 488 MB (852 MB) | measured |
| Grover, new Chrome per document | n/a | 1,948 ms (1,995 ms) | n/a | 716 MB (1,412 MB) | measured |

## Against the SPEC targets

`docs/SPEC.md`: surimono + fulgur at least 2× faster than Grover per document, cold and warm (median of 20 runs), and at least 10× lower peak memory. Ratios are Grover ÷ the other system.

A ratio is shown only when both systems finished every run. "met" or "not met" also needs the number of runs the SPEC asks for on both sides (20 timed runs, 5 memory runs); with fewer it says "indicative".

Memory is judged on PSS, which counts each shared page once. Summed RSS (in brackets) counts the code Chrome's processes share once per process, which inflates Grover's multi-process number; it is shown because it is the usual "RSS of the process tree" number, not because it is the fair one. Grover (chrome-headless-shell) is the leanest and fastest Chrome a Grover user can run; a speed or memory claim should hold against it too, or say that it doesn't.

| Comparison | Cold time | Warm time | Peak memory, cold: PSS (RSS) | Peak memory, warm: PSS (RSS) |
|---|---|---|---|---|
| Grover (Chrome) ÷ surimono + fulgur (Ruby) | 2.3× **met** | 2.3× **met** | 6.0× (12.7×) **not met** | 5.7× (11.0×) **not met** |
| Grover (Chrome) ÷ surimono + fulgur (Rust) | 2.8× **met** | 2.3× **met** | 8.0× (16.8×) **not met** | 8.3× (16.0×) **not met** |
| Grover (Chrome) ÷ fulgur only, Chrome's SVGs (reference point) | 6.0× reference point | 9.0× reference point | 8.9× (18.7×) reference point | 9.0× (17.2×) reference point |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | 1.8× **not met** | 1.8× **not met** | 3.9× (7.1×) **not met** | 3.8× (6.6×) **not met** |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Rust) | 2.1× **met** | 1.8× **not met** | 5.2× (9.5×) **not met** | 5.6× (9.5×) **not met** |
| Grover (chrome-headless-shell) ÷ fulgur only, Chrome's SVGs (reference point) | 4.5× reference point | 7.0× reference point | 5.8× (10.5×) reference point | 6.1× (10.2×) reference point |

The fulgur-only row is a reference point: the same report printed by fulgur without JavaScript (Chrome's SVGs, text not outlined). It is expected to bound surimono's ratios, but it is not a proven bound.

## Details

| System | Cold: setup / render inside the process (median) | Cold CPU time (median) | Warm phases (median) | Pages printed | Processes at peak |
|---|---|---|---|---|---|
| Grover (Chrome) | 52 ms / 1,970 ms | 3,310 ms | grover 1,645 ms | cold 6 / warm 6 | 14 |
| surimono + fulgur (Ruby) | 30 ms / 739 ms | 980 ms | islands 631 ms, pdf 85 ms | cold 6 / warm 6 | 1 |
| surimono + fulgur (Rust) | 17 ms / 748 ms | 830 ms | islands 640 ms, pdf 79 ms | cold 6 / warm 6 | 1 |
| fulgur only, Chrome's SVGs (reference point) | 14 ms / 196 ms | 370 ms | pdf 183 ms | cold 6 / warm 6 | 1 |
| Grover (chrome-headless-shell) | 52 ms / 1,454 ms | 2,255 ms | grover 1,281 ms | cold 6 / warm 6 | 9 |
| Grover, new Chrome per document | — / — | — | grover 1,948 ms | warm 6 | 14 |

Left out of Grover (Chrome)'s warm peak: the Node process that only keeps Chrome open (`bench/grover/browser.cjs`), peak 75 MB PSS (78 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak: the Node process that only keeps Chrome open (`bench/grover/browser.cjs`), peak 74 MB PSS (78 MB RSS). Its Chrome processes are counted.

## Versions

| | |
|---|---|
| Chromium (Grover and the pre-rendered SVGs) | Google Chrome for Testing 153.0.8010.12 |
| chrome-headless-shell (Grover, headless shell) | Google Chrome for Testing 153.0.8010.12 |
| grover gem | 1.2.10 |
| puppeteer | 25.11.0 |
| fulgur gem | 0.40.0 |
| surimono gem | 0.0.0 (native 0.0.0) |
| surimono commit | a3a69e34932bc9221204a0339c3cef2a98626cbb |
| echarts | 6.1.0 |
| vega | 6.4.0 |
| vega-lite | 6.5.0 |
| Ruby | ruby 3.4.12 (2026-10-23 revision 366f5fdc00) +PRISM [x86_64-linux] |
| Node | v22.23.4 |

## Method

- **Grover (Chrome)**: Grover 1.2.10, Puppeteer, Chrome; charts drawn client-side by ECharts/Vega in Chrome. Warm: Ruby process and Chrome reused (browser_ws_endpoint); the Node process that only keeps Chrome open is left out of the memory sum (reported separately).
- **surimono + fulgur (Ruby)**: Surimono::Renderer#render (islands in QuickJS), then the fulgur gem prints the HTML.
- **surimono + fulgur (Rust)**: surimono::Renderer::render_islands, then the fulgur crate 0.40.0, in one Rust process (bench/rust, release build).
- **fulgur only, Chrome's SVGs (reference point)**: The fulgur gem prints the report with Chrome's island SVGs already inlined. No JavaScript runs. Chrome's SVGs keep <text>, which fulgur shapes itself, while surimono outlines text by default, so the PDF step is not identical: a reference point, expected to bound surimono's ratios, NOT a surimono result.
- **Grover (chrome-headless-shell)**: As Grover (Chrome), on chrome-headless-shell of the same build (Puppeteer's headless: "shell"), the leanest Chrome a Grover user can run.
- **Grover, new Chrome per document**: Warm only: Ruby process reused, Grover's default of launching Chrome for every document.
- **Cold** is the wall time of a new process from spawn to exit: Ruby boot and Bundler, setup, one render, the PDF written. The OS page cache is warm after the first run, so cold means a new process, not a cold disk.
- **Warm** is one render inside a long-lived process after 2 untimed renders, timed by the process (monotonic clock); the PDF write is outside the timing.
- **Grover** prints a file:// URL with `wait_until: load` and waits for the islands' done flag. Grover's default for URLs is `networkidle2`, which waits 500 ms after the last request on every document, so Grover is measured at its fastest correct setting. Chrome runs with `--no-sandbox` (root and Ubuntu 24.04 runners need it). Charts are drawn client-side by the same ECharts, Vega and Vega-Lite builds and the same island contract as the Chrome references (`fixtures/tools/lib/islands-browser.js`).
- **Memory** is sampled from `/proc/<pid>/smaps_rollup` for every process in the tree, aiming at one sample every 10 ms (achieved: 11 to 54 ms on average per run; reading many processes takes longer); the harness is a child subreaper, so detached helpers stay counted. A peak shorter than the interval can be missed. A process that exists but can't be read marks the run's memory incomplete instead of counting as zero. `results.json` also has the largest single process.
- **Fonts**: every process gets a fontconfig file that only knows `fixtures/fonts`, so both engines see the same fonts and no system font.
- **surimono's runtime pool**: 4 runtimes (default (CPUs clamped to 1..8), 4 CPUs here). Each pooled runtime keeps its own QuickJS heap, so the pool size shows in peak memory.

