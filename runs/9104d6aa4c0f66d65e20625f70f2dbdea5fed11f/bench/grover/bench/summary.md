# Grover benchmark

Reference report: `bench/report/report.html` (6 pages, 6 chart islands: ECharts, Vega and Vega-Lite; Latin and Japanese; simple and dense). Fonts: Noto Sans and Noto Sans JP from `fixtures/fonts`.

The report is shaped around fulgur 0.40's layout gaps (`docs/COMPAT.md`): flex rows instead of `<table>`, fixed sizes instead of auto-sized flex items, one chart per page. That is right for comparing the same six pages in both engines, but it is not a typical Rails template; a page fulgur can't lay out yet isn't measured here.

Measured on GitHub Actions: INTEL(R) XEON(R) PLATINUM 8573C, 4 cores, 15.6 GB, load average 1.79 at the start and 3.39 at the end. 20 timed runs per cell (cold: interleaved, one process each; warm: one process), 5 separate memory runs for cold and one process for warm.

The bench/rust release build took 2.4 min in this run (not part of any number below).

## Results

Time per document: median (p90). Peak memory: PSS summed over the whole process tree (Chrome's helpers included), median of the memory runs; summed RSS in brackets.

| System | Cold | Warm | Peak memory, cold | Peak memory, warm | Status |
|---|---|---|---|---|---|
| Grover (Chrome) | 1,927 ms (1,952 ms) | 1,431 ms (1,458 ms) | 591 MB (1,284 MB) | 712 MB (1,439 MB) | measured |
| surimono + fulgur (Ruby) | 1,115 ms (1,126 ms) | 613 ms (620 ms) | 146 MB (149 MB) | 172 MB (175 MB) | measured |
| surimono + fulgur (Rust) | 970 ms (980 ms) | 596 ms (612 ms) | 123 MB (126 MB) | 135 MB (138 MB) | measured |
| fulgur only, Chrome's SVGs (reference point) | 284 ms (286 ms) | 130 ms (132 ms) | 62 MB (66 MB) | 83 MB (86 MB) | measured; **reference point, not surimono** (no JavaScript runs; Chrome's SVG text is not outlined) |
| Grover (chrome-headless-shell) | 1,464 ms (1,479 ms) | 1,159 ms (1,174 ms) | 380 MB (720 MB) | 491 MB (862 MB) | measured |
| Grover, new Chrome per document | n/a | 1,748 ms (1,794 ms) | n/a | 718 MB (1,416 MB) | measured |

## Against the SPEC targets

`docs/SPEC.md`: surimono + fulgur at least 2× faster than Grover per document, cold and warm (median of 20 runs), and at least 10× lower peak memory. Ratios are Grover ÷ the other system.

A ratio is shown only when both systems finished every run. "met" or "not met" also needs the number of runs the SPEC asks for on both sides (20 timed runs, 5 memory runs); with fewer it says "indicative".

Memory is judged on PSS, which counts each shared page once. Summed RSS (in brackets) counts the code Chrome's processes share once per process, which inflates Grover's multi-process number; it is shown because it is the usual "RSS of the process tree" number, not because it is the fair one. Grover (chrome-headless-shell) is the leanest and fastest Chrome a Grover user can run; a speed or memory claim should hold against it too, or say that it doesn't.

| Comparison | Cold time | Warm time | Peak memory, cold: PSS (RSS) | Peak memory, warm: PSS (RSS) |
|---|---|---|---|---|
| Grover (Chrome) ÷ surimono + fulgur (Ruby) | 1.7× **not met** | 2.3× **met** | 4.1× (8.6×) **not met** | 4.2× (8.2×) **not met** |
| Grover (Chrome) ÷ surimono + fulgur (Rust) | 2.0× **not met** | 2.4× **met** | 4.8× (10.2×) **not met** | 5.3× (10.4×) **not met** |
| Grover (Chrome) ÷ fulgur only, Chrome's SVGs (reference point) | 6.8× reference point | 11.0× reference point | 9.5× (19.6×) reference point | 8.6× (16.7×) reference point |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | 1.3× **not met** | 1.9× **not met** | 2.6× (4.8×) **not met** | 2.9× (4.9×) **not met** |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Rust) | 1.5× **not met** | 1.9× **not met** | 3.1× (5.7×) **not met** | 3.6× (6.2×) **not met** |
| Grover (chrome-headless-shell) ÷ fulgur only, Chrome's SVGs (reference point) | 5.2× reference point | 8.9× reference point | 6.1× (11.0×) reference point | 5.9× (10.0×) reference point |

The fulgur-only row is a reference point: the same report printed by fulgur without JavaScript (Chrome's SVGs, text not outlined). It is expected to bound surimono's ratios, but it is not a proven bound.

## Details

| System | Cold: setup / render inside the process (median) | Cold CPU time (median) | Warm phases (median) | Pages printed | Processes at peak |
|---|---|---|---|---|---|
| Grover (Chrome) | 43 ms / 1,750 ms | 2,895 ms | grover 1,431 ms | cold 6 / warm 6 | 14 |
| surimono + fulgur (Ruby) | 22 ms / 947 ms | 1,340 ms | islands 531 ms, pdf 81 ms | cold 6 / warm 6 | 1 |
| surimono + fulgur (Rust) | 13 ms / 937 ms | 1,190 ms | islands 522 ms, pdf 74 ms | cold 6 / warm 6 | 1 |
| fulgur only, Chrome's SVGs (reference point) | 9 ms / 144 ms | 280 ms | pdf 130 ms | cold 6 / warm 6 | 1 |
| Grover (chrome-headless-shell) | 43 ms / 1,289 ms | 2,000 ms | grover 1,159 ms | cold 6 / warm 6 | 9 |
| Grover, new Chrome per document | — / — | — | grover 1,748 ms | warm 6 | 14 |

Left out of Grover (Chrome)'s warm peak: the Node process that only keeps Chrome open (`bench/grover/browser.cjs`), peak 78 MB PSS (81 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak: the Node process that only keeps Chrome open (`bench/grover/browser.cjs`), peak 80 MB PSS (84 MB RSS). Its Chrome processes are counted.

## Versions

| | |
|---|---|
| Chromium (Grover and the pre-rendered SVGs) | Google Chrome for Testing 153.0.8010.12 |
| chrome-headless-shell (Grover, headless shell) | Google Chrome for Testing 153.0.8010.12 |
| grover gem | 1.2.10 |
| puppeteer | 25.11.0 |
| fulgur gem | 0.40.0 |
| surimono gem | 0.0.0 (native 0.0.0) |
| surimono commit | 9104d6aa4c0f66d65e20625f70f2dbdea5fed11f |
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
- **Memory** is sampled from `/proc/<pid>/smaps_rollup` for every process in the tree, aiming at one sample every 10 ms (achieved: 10 to 30 ms on average per run; reading many processes takes longer); the harness is a child subreaper, so detached helpers stay counted. A peak shorter than the interval can be missed. A process that exists but can't be read marks the run's memory incomplete instead of counting as zero. `results.json` also has the largest single process.
- **Fonts**: every process gets a fontconfig file that only knows `fixtures/fonts`, so both engines see the same fonts and no system font.
- **surimono's runtime pool**: 4 runtimes (default (CPUs clamped to 1..8), 4 CPUs here). Each pooled runtime keeps its own QuickJS heap, so the pool size shows in peak memory.

