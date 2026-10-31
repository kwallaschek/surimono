# Grover benchmark

Reference report: `bench/report/report.html` (6 pages, 6 chart islands: ECharts, Vega and Vega-Lite; Latin and Japanese; simple and dense). Fonts: Noto Sans and Noto Sans JP from `fixtures/fonts`.

The report is shaped around fulgur 0.40's layout gaps (`docs/COMPAT.md`): flex rows instead of `<table>`, fixed sizes instead of auto-sized flex items, one chart per page. That is right for comparing the same six pages in both engines, but it is not a typical Rails template; a page fulgur can't lay out yet isn't measured here.

Measured on GitHub Actions: AMD EPYC 9V74 80-Core Processor, 4 cores, 15.6 GB, load average 1.05 at the start and 3.50 at the end. 20 timed runs per cell (cold: interleaved, one process each; warm: one process), 5 separate memory runs for cold and one process for warm.

The bench/rust release build took 3.3 min in this run (not part of any number below).

## Results

Time per document: median (p90). Peak memory: PSS summed over the whole process tree (Chrome's helpers included), median of the memory runs; summed RSS in brackets.

| System | Cold | Warm | Peak memory, cold | Peak memory, warm | Status |
|---|---|---|---|---|---|
| Grover (Chrome) | 1,680 ms (1,698 ms) | 1,203 ms (1,260 ms) | 584 MB (1,286 MB) | 719 MB (1,437 MB) | measured |
| surimono + fulgur (Ruby) | — | — | — | — | **not measured** (unavailable in cold): surimono gem not built (cannot load such file -- surimono/surimono); build it with `bundle exec rake compile` in bindings/ruby |
| surimono + fulgur (Rust) | — | — | — | — | **not measured** (unavailable in cold): surimono 0.0.0 can't render this report yet: renderers echarts, vega, vega-lite not available (it has: surimono-stub) |
| fulgur only, Chrome's SVGs (reference point) | 282 ms (284 ms) | 138 ms (142 ms) | 57 MB (60 MB) | 81 MB (84 MB) | measured; **reference point, not surimono** (no JavaScript runs; Chrome's SVG text is not outlined) |
| Grover (chrome-headless-shell) | 1,262 ms (1,283 ms) | 967 ms (982 ms) | 378 MB (720 MB) | 485 MB (850 MB) | measured |
| Grover, new Chrome per document | n/a | 1,497 ms (1,543 ms) | n/a | 718 MB (1,417 MB) | measured |

## Against the SPEC targets

`docs/SPEC.md`: surimono + fulgur at least 2× faster than Grover per document, cold and warm (median of 20 runs), and at least 10× lower peak memory. Ratios are Grover ÷ the other system.

A ratio is shown only when both systems finished every run. "met" or "not met" also needs the number of runs the SPEC asks for on both sides (20 timed runs, 5 memory runs); with fewer it says "indicative".

Memory is judged on PSS, which counts each shared page once. Summed RSS (in brackets) counts the code Chrome's processes share once per process, which inflates Grover's multi-process number; it is shown because it is the usual "RSS of the process tree" number, not because it is the fair one. Grover (chrome-headless-shell) is the leanest Chrome a Grover user can run; a memory claim should hold against it too, or say that it doesn't.

| Comparison | Cold time | Warm time | Peak memory, cold: PSS (RSS) | Peak memory, warm: PSS (RSS) |
|---|---|---|---|---|
| Grover (Chrome) ÷ surimono + fulgur (Ruby) | not measured (surimono + fulgur (Ruby) not fully measured) | — | — | — |
| Grover (Chrome) ÷ surimono + fulgur (Rust) | not measured (surimono + fulgur (Rust) not fully measured) | — | — | — |
| Grover (Chrome) ÷ fulgur only, Chrome's SVGs (reference point) | 6.0× reference point | 8.7× reference point | 10.3× (21.4×) reference point | 8.9× (17.1×) reference point |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | not measured (surimono + fulgur (Ruby) not fully measured) | — | — | — |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Rust) | not measured (surimono + fulgur (Rust) not fully measured) | — | — | — |
| Grover (chrome-headless-shell) ÷ fulgur only, Chrome's SVGs (reference point) | 4.5× reference point | 7.0× reference point | 6.6× (12.0×) reference point | 6.0× (10.1×) reference point |

**No surimono ratio is measured yet:** the rows above say why. The fulgur-only row prints the same report with Chrome's SVGs already inlined, so it does what the surimono path does except running the chart libraries. It is a reference point, not a result: Chrome's SVGs keep `<text>`, which fulgur shapes itself, while surimono outlines text by default, so the PDF step is not the same. It is expected to bound surimono's ratios.

## Details

| System | Cold: setup / render inside the process (median) | Cold CPU time (median) | Warm phases (median) | Pages printed | Processes at peak |
|---|---|---|---|---|---|
| Grover (Chrome) | 39 ms / 1,521 ms | 2,515 ms | grover 1,203 ms | cold 6 / warm 6 | 14 |
| fulgur only, Chrome's SVGs (reference point) | 9 ms / 156 ms | 280 ms | pdf 138 ms | cold 6 / warm 6 | 1 |
| Grover (chrome-headless-shell) | 40 ms / 1,104 ms | 1,720 ms | grover 967 ms | cold 6 / warm 6 | 9 |
| Grover, new Chrome per document | — / — | — | grover 1,497 ms | warm 6 | 14 |

Left out of Grover (Chrome)'s warm peak: the Node process that only keeps Chrome open (`bench/grover/browser.cjs`), peak 74 MB PSS (78 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak: the Node process that only keeps Chrome open (`bench/grover/browser.cjs`), peak 74 MB PSS (78 MB RSS). Its Chrome processes are counted.

## Versions

| | |
|---|---|
| Chromium (Grover and the pre-rendered SVGs) | Google Chrome for Testing 153.0.8010.12 |
| chrome-headless-shell (Grover, headless shell) | Google Chrome for Testing 153.0.8010.12 |
| grover gem | 1.2.10 |
| puppeteer | 25.11.0 |
| fulgur gem | 0.40.0 |
| surimono gem |  |
| surimono commit | 34d1c1be9aa3a5501dcd5af3a3a96fc194f56bb5 |
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
- **Memory** is sampled from `/proc/<pid>/smaps_rollup` for every process in the tree, aiming at one sample every 10 ms (achieved: 11 to 48 ms on average per run; reading many processes takes longer); the harness is a child subreaper, so detached helpers stay counted. A peak shorter than the interval can be missed. A process that exists but can't be read marks the run's memory incomplete instead of counting as zero. `results.json` also has the largest single process.
- **Fonts**: every process gets a fontconfig file that only knows `fixtures/fonts`, so both engines see the same fonts and no system font.

