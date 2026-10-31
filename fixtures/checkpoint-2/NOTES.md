# Checkpoint 2: where each result comes from

`index.md`, `checkpoint.json` and the per-fixture folders are written by
`fixtures/tools/checkpoint.mjs`; `fulgur-baseline.*` by `baseline.mjs`;
`hostile-results.json` by `cargo test --release -p surimono-islands --test hostile`
with `SURIMONO_HOSTILE_RESULTS` set; `gfx-svg-*.log` by `reference.sh`. This file is
hand-written: it says which machine and which Chromium produced what.

**This run.** The CLI was built in release mode from main at 99c363cd (after #47's
time-accurate deadlines and engine patch level 3, and #62's fork generation) and
run with `checkpoint.mjs --surimono <target>/release/surimono --commit 99c363cd…
--baseline <CI renders of 88d599f4>`. Paths in the committed JSON are relabelled
after the run: the output directory reads `checkpoint-out/` and the CLI
`target/release/surimono` (the fleet's shared target directory), so no machine
path is committed; the CLI's sha256 identifies it. Every island and page output
is byte-identical to the 6eb38139 run's (same `sha256` in `checkpoint.json`),
and so is each of the 35 side-by-side images before palette reduction, so the
committed 256-colour PNGs are unchanged; only the producer stamps and render
times differ.

**Two Chromiums, one build.** Every Chrome result here comes from Chromium
153.0.8010.12 (the build Playwright 1.63.0 pins), but from two machines:

- **The gate and every surimono comparison ran on the fleet's Linux machine**
  (Ubuntu 24.04, poppler 24.02.0). Chrome printing surimono's island SVGs (the
  gate), fulgur printing them, and fulgur printing surimono's page were all
  rasterised there and compared with the committed references.
- **The committed references and fulgur's floor (`fulgur-baseline.md`) come
  from GitHub's runner**: CI's reference job on 88d599f4
  (`ci-results:runs/88d599f48f0eb73d34dc602e68e1bd87c4b08931/reference/chrome/`,
  exit 0), whose own render reproduced all 11 fixtures' references `identical`.
  Nothing under `fixtures/islands/`, `fixtures/fonts/` or the comparison code has
  changed since then, so those renders still stand for the current references.

**Why the floor isn't measured locally.** On the fleet machine the same
Chromium build does not reproduce two references exactly: in
`reference.sh`'s check, echarts-line-dense-latin differs by 65 px and
echarts-two-charts-latin by 38 px (pixelmatch pixels, on the page and on island 0
alike, against about 36,000 and 65,000 inked pixels on those islands; island 1 of
echarts-two-charts-latin reproduces pixel for pixel). The difference starts in
Chrome's own SVG, before anything is printed: ECharts places some axis labels
with metrics it measures through the browser, and the local Chromium gives a
label offset of `y="4.999992370605469"` (and `6.0000457763671875`) where CI's
gives exactly `5` (and `6`). Why one build measures differently on two machines
isn't established; a CPU-dependent floating-point path in Chromium's text
measurement is the likely cause, not anything in the fixtures. The size is far
below what the metric can see: every near-match, colour and hue window of both
fixtures scores 0.0% (`worst_window` 0), so the gate's verdicts don't depend on
which machine printed them. fulgur's floor is taken from CI's renders anyway,
because it is fulgur printing Chrome's SVG, and that should be the SVG the
references were made from, not a locally shifted one.

surimono's own SVGs don't take metrics from Chrome (gfx measures text with the
fixture fonts), so this difference never enters a surimono render.

**Hostile results.** `hostile-results.json` names commit 52d76a39, the harness's
`git rev-parse HEAD`: this branch's first commit, which is 99c363cd plus fixture
tooling only (nothing under `crates/` or `vendor/` differs). Every case ends with
the same outcome, output and message as on 6eb38139 except
`native-sort-typed-array`, whose manifest accepts four outcomes: sorting 48 MB in
one native call takes about 1 to 2.4 s here depending on what else the machine is
running, so it either finishes (`Ok`, "sorted") or hits the 2 s time limit.
Across the three release runs so far it was `TimeLimit` at 2083 ms (6eb38139),
`TimeLimit` at 2391 ms and `Ok` at 1031 ms (99c363cd, the last one committed).
