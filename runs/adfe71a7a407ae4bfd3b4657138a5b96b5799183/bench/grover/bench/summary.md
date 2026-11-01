# Grover benchmark

Reference report: `bench/report/report.html` (6 pages, 6 chart islands: ECharts, Vega and Vega-Lite; Latin and Japanese; simple and dense). Fonts: Noto Sans and Noto Sans JP from `fixtures/fonts`.

The report is shaped around fulgur 0.40's layout gaps (`docs/COMPAT.md`): flex rows instead of `<table>`, fixed sizes instead of auto-sized flex items, one chart per page. That is right for comparing the same six pages in both engines, but it is not a typical Rails template; a page fulgur can't lay out yet isn't measured here.

Measured on GitHub Actions: Intel(R) Xeon(R) 6973P-C, 4 cores, 15.6 GB, load average 2.45 at the start and 3.19 at the end. 20 timed runs per cell (cold: interleaved, one process each; warm: one process), 5 separate memory runs for cold and one process for warm.

The bench/rust release build took 1.9 min in this run (not part of any number below).

## Results

Time per document: median (p90). Peak memory: PSS summed over the whole process tree (Chrome's helpers included), median of the memory runs; summed RSS in brackets.

| System | Cold | Warm | Peak memory, cold | Peak memory, warm | Status |
|---|---|---|---|---|---|
| Grover (Chrome) | 1,648 ms (1,698 ms) | 1,217 ms (1,257 ms) | 589 MB (1,283 MB) | 702 MB (1,423 MB) | measured |
| surimono + fulgur (Ruby) | 655 ms (692 ms) | 536 ms (550 ms) | 100 MB (104 MB) | 125 MB (128 MB) | measured |
| surimono + fulgur (Rust) | 548 ms (586 ms) | 539 ms (550 ms) | 83 MB (86 MB) | 87 MB (90 MB) | measured |
| fulgur only, Chrome's SVGs (reference point) | 235 ms (253 ms) | 113 ms (116 ms) | 60 MB (63 MB) | 81 MB (84 MB) | measured; **reference point, not surimono** (no JavaScript runs; Chrome's SVG text is not outlined) |
| Grover (chrome-headless-shell) | 1,220 ms (1,259 ms) | 944 ms (993 ms) | 382 MB (724 MB) | 489 MB (858 MB) | measured |
| Grover, new Chrome per document | n/a | 1,433 ms (1,501 ms) | n/a | 712 MB (1,415 MB) | measured |

## Against the SPEC targets

`docs/SPEC.md`: surimono + fulgur at least 2× faster than Grover per document, cold and warm (median of 20 runs), and at least 10× lower peak memory. Ratios are Grover ÷ the other system.

A ratio is shown only when both systems finished every run. "met" or "not met" also needs the number of runs the SPEC asks for on both sides (20 timed runs, 5 memory runs); with fewer it says "indicative".

Memory is judged on PSS, which counts each shared page once. Summed RSS (in brackets) counts the code Chrome's processes share once per process, which inflates Grover's multi-process number; it is shown because it is the usual "RSS of the process tree" number, not because it is the fair one. Grover (chrome-headless-shell) is the leanest and fastest Chrome a Grover user can run; a speed or memory claim should hold against it too, or say that it doesn't.

| Comparison | Cold time | Warm time | Peak memory, cold: PSS (RSS) | Peak memory, warm: PSS (RSS) |
|---|---|---|---|---|
| Grover (Chrome) ÷ surimono + fulgur (Ruby) | 2.5× **met** | 2.3× **met** | 5.9× (12.4×) **not met** | 5.6× (11.1×) **not met** |
| Grover (Chrome) ÷ surimono + fulgur (Rust) | 3.0× **met** | 2.3× **met** | 7.1× (15.0×) **not met** | 8.0× (15.8×) **not met** |
| Grover (Chrome) ÷ fulgur only, Chrome's SVGs (reference point) | 7.0× reference point | 10.8× reference point | 9.9× (20.4×) reference point | 8.7× (16.9×) reference point |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Ruby) | 1.9× **not met** | 1.8× **not met** | 3.8× (7.0×) **not met** | 3.9× (6.7×) **not met** |
| Grover (chrome-headless-shell) ÷ surimono + fulgur (Rust) | 2.2× **met** | 1.8× **not met** | 4.6× (8.5×) **not met** | 5.6× (9.5×) **not met** |
| Grover (chrome-headless-shell) ÷ fulgur only, Chrome's SVGs (reference point) | 5.2× reference point | 8.4× reference point | 6.4× (11.5×) reference point | 6.0× (10.2×) reference point |

The fulgur-only row is a reference point: the same report printed by fulgur without JavaScript (Chrome's SVGs, text not outlined). It is expected to bound surimono's ratios, but it is not a proven bound.

## Details

| System | Cold: setup / render inside the process (median) | Cold CPU time (median) | Warm phases (median) | Pages printed | Processes at peak |
|---|---|---|---|---|---|
| Grover (Chrome) | 35 ms / 1,495 ms | 2,430 ms | grover 1,217 ms | cold 6 / warm 6 | 14 |
| surimono + fulgur (Ruby) | 18 ms / 523 ms | 700 ms | islands 466 ms, pdf 68 ms | cold 6 / warm 6 | 1 |
| surimono + fulgur (Rust) | 9 ms / 527 ms | 590 ms | islands 472 ms, pdf 66 ms | cold 6 / warm 6 | 1 |
| fulgur only, Chrome's SVGs (reference point) | 8 ms / 121 ms | 230 ms | pdf 113 ms | cold 6 / warm 6 | 1 |
| Grover (chrome-headless-shell) | 34 ms / 1,080 ms | 1,650 ms | grover 944 ms | cold 6 / warm 6 | 9 |
| Grover, new Chrome per document | — / — | — | grover 1,433 ms | warm 6 | 14 |

Left out of Grover (Chrome)'s warm peak: the Node process that only keeps Chrome open (`bench/grover/browser.cjs`), peak 75 MB PSS (79 MB RSS). Its Chrome processes are counted.

Left out of Grover (chrome-headless-shell)'s warm peak: the Node process that only keeps Chrome open (`bench/grover/browser.cjs`), peak 73 MB PSS (77 MB RSS). Its Chrome processes are counted.

## Versions

| | |
|---|---|
| Chromium (Grover and the pre-rendered SVGs) | Google Chrome for Testing 153.0.8010.12 |
| chrome-headless-shell (Grover, headless shell) | Google Chrome for Testing 153.0.8010.12 |
| grover gem | 1.2.10 |
| puppeteer | 25.11.0 |
| fulgur gem | 0.40.0 |
| surimono gem | 0.0.0 (native 0.0.0) |
| surimono commit | adfe71a7a407ae4bfd3b4657138a5b96b5799183 |
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

