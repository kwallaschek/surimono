#!/usr/bin/env node
// The printer matrix: the same pages printed by several printers (versions of
// fulgur, or anything that takes `fulgur render`'s command line, such as
// raikiri-print), each scored against the committed Chrome references by the one metric
// and pass rule of fixtures/README.md, with each print's time and peak memory.
//
//   node printers.mjs --out DIR --surimono CLI --check BIN [--commit SHA] [--runs 5]
//                     --printer LABEL=FULGUR [--printer LABEL=FULGUR ...] [fixture ...]
//
// CLI is the surimono CLI and BIN surimono-page-check, both built from --commit.
// For each printer this runs the two existing pipelines with that fulgur and
// nothing else changed:
//
//   checkpoint.mjs --no-chrome --fulgur FULGUR      the island fixtures: surimono's page,
//                                                   and each island's SVG alone
//   print-pages.mjs --keep-prints --fulgur FULGUR   the real-world pages: surimono's output,
//                                                   and Chrome's own DOM (the floor)
//
// into DIR/.runs/<LABEL>/, then reads their scores. It scores nothing itself and
// changes no reference and no threshold. The first printer is the one the others
// are compared with. Naming fixtures runs only those (of either kind). DIR must be
// outside fixtures/.
//
// Time and memory are measured afterwards, page by page, with the printers taking
// turns (bench/tools/measure.py --commands): a round prints the page once with each
// printer, one round that isn't measured, then --runs timed rounds, then --runs
// sampled ones. Who starts moves on by one every round and every page, so no printer
// is always first and none has the machine at another hour than the others.
// Writes:
//
//   DIR/printers.json   every cell of every printer: verdict, scores, the PDF's size and
//                       sha256, and each print's time and memory with every single run
//   DIR/index.md        rows: pages; columns: the printers
//   DIR/<fixture>/<cell>.png   Chrome | printer 1 | printer 2 | …, only for cells whose
//                              verdict or scores differ between printers
//
// Paths in what it writes are relabelled (DIR as printers-out/, the repository
// as nothing), so the results can be committed as they are. Exits 0 when
// everything ran (a failing comparison is a result), 1 when a pipeline failed or
// surimono's output differs between two printers' runs, 2 on usage errors.

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { parseArgs } from "node:util";
import { FIXTURES_DIR, TOOLS_DIR, listAllFixtures, popplerVersion } from "./lib/fixtures.mjs";
import { PASS_COLOUR_WINDOW, PASS_HUE_WINDOW, PASS_WORST_WINDOW, referenceFiles } from "./lib/compare.mjs";
import { failedChecks, passes, pct } from "./lib/report.mjs";
import { pixelsIdentical, readPng, sideBySide, writePng } from "./lib/png.mjs";
import { MEASURE_INTERVAL_MS, fulgurMeasureTurns, pdfIdentity } from "./lib/fulgur.mjs";

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    out: { type: "string" },
    surimono: { type: "string" },
    check: { type: "string" },
    commit: { type: "string" },
    runs: { type: "string", default: "5" },
    printer: { type: "string", multiple: true },
  },
});
const usage = (why) => {
  if (why) console.error(why);
  console.error("usage: node printers.mjs --out DIR --surimono CLI --check BIN [--commit SHA] [--runs 5] --printer LABEL=FULGUR [--printer LABEL=FULGUR ...] [fixture ...]");
  process.exit(2);
};
const runs = Number(values.runs);
if (!values.out || !values.surimono || !values.check || !values.printer?.length || !Number.isInteger(runs) || runs < 1) usage();
const printers = values.printer.map((p) => {
  const at = p.indexOf("=");
  if (at < 1 || !/^[\w.+-]+$/.test(p.slice(0, at))) usage(`--printer ${p}: expected LABEL=FULGUR, the label of letters, digits, ".", "+", "-" and "_"`);
  return { label: p.slice(0, at), bin: path.resolve(p.slice(at + 1)) };
});
if (new Set(printers.map((p) => p.label)).size !== printers.length) usage("two printers have the same label");
const outRoot = path.resolve(values.out);
const inside = path.relative(FIXTURES_DIR, outRoot);
if (!inside.startsWith("..") && !path.isAbsolute(inside)) {
  console.error(`--out ${outRoot} is inside fixtures/; write results elsewhere`);
  process.exit(2);
}
const REPO = path.resolve(FIXTURES_DIR, "..");
let chosen;
try {
  chosen = listAllFixtures(positionals);
} catch (e) {
  usage(e.message);
}
/** The fixtures of one kind this run prints. */
const namesOf = (kind) => chosen.filter((f) => f.kind === kind).map((f) => f.name);
const sha256 = (file) => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
const readJson = (file) => JSON.parse(fs.readFileSync(file, "utf8"));

// Without the rasteriser every cell would be an error and no two printers would differ.
try {
  popplerVersion();
} catch (e) {
  console.error(`${e.message}: printers.mjs compares each printed page as an image`);
  process.exit(1);
}
for (const p of printers) {
  const v = spawnSync(p.bin, ["--version"], { encoding: "utf8" });
  if (v.status !== 0) {
    console.error(`${p.bin} --version failed: ${v.error?.message ?? v.stderr}`);
    process.exit(1);
  }
  // What it calls itself: "fulgur 0.40.0", "fulgur-raikiri 0.41.0".
  p.version = v.stdout.trim().split("\n")[0];
  p.sha256 = sha256(p.bin);
}

/** One of the two pipelines with one printer; its output stays in DIR/.runs/<label>/<kind>. */
function pipeline(printer, kind) {
  const dir = path.join(outRoot, ".runs", printer.label, kind);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const common = ["--out", dir, "--fulgur", printer.bin, ...(values.commit ? ["--commit", values.commit] : []), ...namesOf(kind)];
  const args =
    kind === "islands"
      ? [path.join(TOOLS_DIR, "checkpoint.mjs"), "--surimono", path.resolve(values.surimono), "--no-chrome", ...common]
      : [path.join(TOOLS_DIR, "print-pages.mjs"), "--check", path.resolve(values.check), "--keep-prints", ...common];
  console.log(`\n==== ${printer.label} (${printer.version}): ${kind}`);
  const r = spawnSync(process.execPath, args, { encoding: "utf8", maxBuffer: 1 << 28 });
  fs.writeFileSync(path.join(dir, "log.txt"), `${r.stdout}${r.stderr}`);
  if (r.status !== 0) {
    console.error(`${path.basename(args[0])} exited ${r.status ?? r.signal} with ${printer.label}:\n${`${r.stdout}${r.stderr}`.split("\n").slice(-30).join("\n")}`);
    process.exit(1);
  }
  return dir;
}

/** The PDF compare.mjs left in an island's folder, when a pipeline didn't say what it printed. */
function pdfOf(dir) {
  const file = path.join(dir, "surimono.pdf");
  return fs.existsSync(file) ? pdfIdentity(file) : undefined;
}

/** What a cell keeps of a comparison's score.json: the verdict and its numbers, no paths. */
function cellOf(dir, printed, input) {
  const file = path.join(dir, "score.json");
  const s = fs.existsSync(file) ? readJson(file) : { pass: false, error: "no score.json", pages: [] };
  const pages = Array.isArray(s.pages) ? s.pages : [];
  const raster = fs.existsSync(dir) ? fs.readdirSync(dir).filter((f) => /^surimono-\d+\.png$/.test(f)).sort((a, b) => parseInt(a.slice(9), 10) - parseInt(b.slice(9), 10)).map((f) => path.join(dir, f)) : [];
  return {
    pass: passes(s),
    failed: s.error ? ["no output"] : failedChecks(s),
    shape: s.error ? null : (s.worst_window ?? null),
    colour: s.error ? null : (s.worst_colour_window ?? null),
    hue: s.error ? null : (s.worst_hue_window ?? null),
    pixelmatch_ink: s.error ? null : (s.worst_diff_ratio_ink ?? null),
    pages: s.page_count ?? null,
    size: pages[0]?.actual_size ?? null,
    chrome_size: pages[0]?.expected_size ?? null,
    text: s.text ? s.text.pass === true : undefined,
    font: s.font ? s.font.pass === true : undefined,
    // A cell without a print says only that: why (the printer's exit, a signal, a
    // timeout, a PDF nothing can read) is kept in DIR/.private/failures.json, which
    // isn't for committing, because how a printer dies on an input is its project's
    // to hear first.
    error: s.error ? String(s.error) : undefined,
    no_output: s.error ? true : undefined,
    printer_exit: s.printer,
    pdf: printed ?? pdfOf(dir),
    input,
    raster,
  };
}

/** Every cell one printer printed: { key: { fixture, kind, what, ...cell } } in table order. */
function cellsOf(printer) {
  const cells = new Map();
  const outputs = {};
  const islands = namesOf("islands").length ? pipeline(printer, "islands") : null;
  for (const row of islands ? readJson(path.join(islands, "checkpoint.json")).rows : []) {
    outputs[`islands/${row.fixture}`] = row.sha256 ?? null;
    cells.set(`islands/${row.fixture}/page`, { set: "islands", fixture: row.fixture, what: "page", ...cellOf(path.join(islands, row.fixture, "page"), row.printed, { html: path.join(islands, row.fixture, "surimono.html"), fonts: [] }) });
    row.islands.forEach((_, i) => cells.set(`islands/${row.fixture}/island-${i}`, { set: "islands", fixture: row.fixture, what: `island-${i}`, ...cellOf(path.join(islands, row.fixture, `island-${i}-fulgur`)) }));
  }
  const pages = namesOf("real-world").length ? pipeline(printer, "real-world") : null;
  for (const row of pages ? readJson(path.join(pages, "print-pages.json")).rows : []) {
    outputs[`real-world/${row.fixture}`] = row.sha256 ?? null;
    cells.set(`real-world/${row.fixture}/page`, { set: "real-world", fixture: row.fixture, what: "page", ...cellOf(path.join(pages, row.fixture, "page"), row.page.printed, row.page.input) });
    cells.set(`real-world/${row.fixture}/floor`, { set: "real-world", fixture: row.fixture, what: "floor", ...cellOf(path.join(pages, row.fixture, "floor"), row.floor.printed, row.floor.input) });
  }
  return { cells, outputs };
}

fs.mkdirSync(outRoot, { recursive: true });
const results = printers.map((p) => ({ printer: p, ...cellsOf(p) }));

// One surimono, so every printer must have been given the same bytes.
const moved = Object.keys(results[0].outputs).filter((k) => results.some((r) => r.outputs[k] !== results[0].outputs[k]));
if (moved.length) {
  console.error(`surimono's output differs between two printers' runs: ${moved.join(", ")}`);
  process.exit(1);
}

// Time and memory: each page printed again by every printer in turns. The first
// printer's kept input is what all of them print; it is the same bytes for each.
const measureDir = path.join(outRoot, ".measure");
fs.rmSync(measureDir, { recursive: true, force: true });
let measuredPages = 0;
const measureFailures = [];
for (const [key, first] of results[0].cells) {
  const cells = results.map((r) => r.cells.get(key));
  if (!first.input) continue; // an island alone: printed by compare.mjs, not timed
  // Only the printers that printed this page take part; the others have nothing to time.
  const printed = results.filter((r, i) => cells[i].pdf && !cells[i].error).map((r) => r.printer);
  for (const c of cells) c.print = { not_measured: true };
  if (!printed.length) continue;
  console.log(`measuring ${key}`);
  const m = fulgurMeasureTurns(printed, first.input.html, measureDir, first.input.fonts, runs, measuredPages++);
  if (m.error) {
    // A print that worked when it was scored died when it was timed: nobody's numbers for this page.
    measureFailures.push({ key, printer: m.label ?? null, error: m.error, printer_exit: { status: m.status, signal: m.signal, timed_out: m.timed_out } });
    continue;
  }
  results.forEach((r, i) => {
    if (m.prints[r.printer.label]) cells[i].print = m.prints[r.printer.label];
  });
  first.print_order = { warmup_rounds: m.warmup_rounds, timed: m.timed_order, sampled: m.sampled_order };
}
fs.rmSync(measureDir, { recursive: true, force: true });

/** The scores a reader sees: two cells with the same signature read the same in the table. */
const signature = (c) => JSON.stringify([c.pass, c.failed, pct(c.shape), pct(c.colour), pct(c.hue), c.pages, c.size, c.text, c.font, Boolean(c.error ?? c.no_output)]);

/** The Chrome reference pages a cell is compared with. */
function referencesOf(cell) {
  const refs = referenceFiles(path.join(FIXTURES_DIR, cell.set, cell.fixture));
  return cell.what.startsWith("island-") ? [refs.islands[Number(cell.what.slice(7))]] : refs.pages;
}

const keys = [...results[0].cells.keys()];
/** Every print or comparison that died, with its reason: for DIR/.private, never for the results. */
const failures = [...measureFailures];
const rows = keys.map((key) => {
  const cells = results.map((r) => r.cells.get(key));
  const first = cells[0];
  const row = { key, set: first.set, fixture: first.fixture, what: first.what, differs: cells.some((c) => signature(c) !== signature(first)) };
  // Who printed first in each measured round (the others follow in the order of `printers`, wrapping).
  if (first.print_order) row.print_order = { warmup_rounds: first.print_order.warmup_rounds, timed_first: first.print_order.timed.map((o) => o[0]), sampled_first: first.print_order.sampled.map((o) => o[0]) };
  // Whether every printer put the same pixels on the page, whatever the scores say.
  row.pixels_identical =
    cells.every((c) => c.raster.length && c.raster.length === first.raster.length) &&
    cells.every((c) => c.raster.every((f, i) => pixelsIdentical(readPng(f), readPng(first.raster[i]))));
  // And whether they wrote the same PDF, byte for byte.
  row.pdf_identical = cells.every((c) => c.pdf && c.pdf.sha256 === first.pdf?.sha256);
  if (row.differs) {
    const refs = referencesOf(first);
    const n = Math.max(refs.length, ...cells.map((c) => c.raster.length));
    row.images = [];
    for (let i = 0; i < n; i++) {
      const panels = [refs[i], ...cells.map((c) => c.raster[i])].filter(Boolean).map((f) => readPng(f));
      const rel = path.join(first.fixture, `${first.what}${n > 1 ? `-${i + 1}` : ""}.png`);
      fs.mkdirSync(path.join(outRoot, first.fixture), { recursive: true });
      writePng(path.join(outRoot, rel), sideBySide(panels));
      row.images.push(rel);
    }
  }
  cells.forEach((c, i) => {
    if (c.error) failures.push({ key, printer: results[i].printer.label, error: c.error, printer_exit: c.printer_exit ?? null });
  });
  row.printers = Object.fromEntries(results.map((r, i) => [r.printer.label, (({ raster, set, fixture, what, input, print_order, error, printer_exit, ...kept }) => kept)(cells[i])]));
  return row;
});

// ------------------------------------------------------------------ report

const labels = printers.map((p) => p.label);
const ms = (x) => (typeof x === "number" ? `${x.toFixed(0)} ms` : "—");
const mb = (x) => (typeof x === "number" ? `${x.toFixed(1)} MB` : "—");
/** "pass · 0.0%", or "**FAIL** · 67.5% (colour 84.8%; hue 100.0%)": the verdict, the shape score, then what else failed. */
function cellText(c) {
  if (c.no_output) return "**FAIL** (no output)";
  if (c.pass) return `pass · ${pct(c.shape)}`;
  const px = (size) => (Array.isArray(size) ? size.join("×") : String(size));
  const detail = { colour: `colour ${pct(c.colour)}`, hue: `hue ${pct(c.hue)}`, size: `size ${px(c.size)}, Chrome's ${px(c.chrome_size)}`, page: "page count" };
  const other = c.failed.filter((f) => f !== "shape").map((f) => detail[f] ?? f);
  return `**FAIL** · ${pct(c.shape)}${other.length ? ` (${other.join("; ")})` : ""}`;
}
const WHAT = { page: "surimono's page", floor: "Chrome's DOM (floor)" };
const what = (w) => WHAT[w] ?? `island ${w.slice(7)} alone`;
const image = (row) => (row.images ? row.images.map((f) => `[image](${f})`).join(" ") : "");
const print = (row, label, field) => row.printers[label].print?.[field]?.median;

function table(set, measured) {
  const body = rows.filter((r) => r.set === set && (measured ? !r.what.startsWith("island-") : r.what.startsWith("island-")));
  if (!body.length) return ["None in this run."];
  const head = ["Page", "Printed", ...labels, ...(measured ? [...labels.map((l) => `Time, ${l}`), ...labels.map((l) => `Peak PSS, ${l}`)] : []), "Same pixels", "Same PDF", "Image"];
  return [
    `| ${head.join(" | ")} |`,
    `|${head.map(() => "---").join("|")}|`,
    ...body.map((r) =>
      `| ${[r.fixture, what(r.what), ...labels.map((l) => cellText(r.printers[l])), ...(measured ? [...labels.map((l) => ms(print(r, l, "wall_ms"))), ...labels.map((l) => mb(print(r, l, "peak_pss_mb")))] : []), r.pixels_identical ? "yes" : "no", r.pdf_identical ? "yes" : "no", image(r)].join(" | ")} |`,
    ),
  ];
}

/** Sums and medians over the measured prints (pages and floors) every printer finished. */
const measuredRows = rows.filter((r) => labels.every((l) => typeof print(r, l, "wall_ms") === "number" && typeof print(r, l, "peak_pss_mb") === "number"));
const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  return s.length ? (s.length % 2 ? s[(s.length - 1) / 2] : (s[s.length / 2 - 1] + s[s.length / 2]) / 2) : null;
};
const totals = Object.fromEntries(
  labels.map((l) => {
    const t = measuredRows.map((r) => print(r, l, "wall_ms"));
    const m = measuredRows.map((r) => print(r, l, "peak_pss_mb"));
    return [l, { prints: measuredRows.length, wall_ms_sum: Math.round(t.reduce((a, b) => a + b, 0) * 10) / 10, wall_ms_median: median(t), peak_pss_mb_median: median(m), peak_pss_mb_max: m.length ? Math.max(...m) : null }];
  }),
);
/**
 * One printer against the first, on what was measured: per page, the difference of
 * the two medians; and per page whether the two printers' single runs overlap at all
 * (when every run of one lies outside the other's range, the page shows a difference
 * that isn't the spread of the runs).
 */
const round1 = (x) => Math.round(x * 10) / 10;
function against(label, field) {
  const base = labels[0];
  const pages = measuredRows.map((r) => {
    const a = r.printers[base].print;
    const b = r.printers[label].print;
    const [ra, rb] = [a[`${field}_runs`], b[`${field}_runs`]];
    return { key: r.key, diff: round1(b[field].median - a[field].median), below: Math.max(...rb) < Math.min(...ra), above: Math.min(...rb) > Math.max(...ra) };
  });
  const diffs = pages.map((x) => x.diff);
  const at = (d) => pages.find((x) => x.diff === d)?.key;
  return {
    pages: pages.length,
    median_of_first: median(measuredRows.map((r) => r.printers[base].print[field].median)),
    median_of_this: median(measuredRows.map((r) => r.printers[label].print[field].median)),
    per_page_difference: { min: Math.min(...diffs), median: round1(median(diffs)), max: Math.max(...diffs), min_at: at(Math.min(...diffs)), max_at: at(Math.max(...diffs)) },
    pages_lower: diffs.filter((d) => d < 0).length,
    pages_equal: diffs.filter((d) => d === 0).length,
    pages_higher: diffs.filter((d) => d > 0).length,
    pages_with_every_run_lower: pages.filter((x) => x.below).length,
    pages_with_every_run_higher: pages.filter((x) => x.above).length,
  };
}
const comparisons = measuredRows.length ? Object.fromEntries(labels.slice(1).map((l) => [l, { wall_ms: against(l, "wall_ms"), peak_pss_mb: against(l, "peak_pss_mb") }])) : {};
const signed = (x, unit) => `${x > 0 ? "+" : x < 0 ? "−" : ""}${Math.abs(x).toFixed(1)} ${unit}`;
function comparisonTable() {
  if (!Object.keys(comparisons).length) return [];
  const line = (l, name, c, unit) =>
    `| ${l} | ${name} | ${c.median_of_first.toFixed(1)} ${unit} | ${c.median_of_this.toFixed(1)} ${unit} | ${signed(c.per_page_difference.min, unit)} | ${signed(c.per_page_difference.median, unit)} | ${signed(c.per_page_difference.max, unit)} | ${c.pages_lower} / ${c.pages_equal} / ${c.pages_higher} | ${c.pages_with_every_run_lower} | ${c.pages_with_every_run_higher} |`;
  return [
    `## Time and memory against ${labels[0]}`,
    "",
    `Over the ${measuredRows.length} measured prints. A page's difference is the printer's median minus ${labels[0]}'s. The last two columns count the pages where every single run of the printer lies below, or above, every run of ${labels[0]}: on the other pages the runs overlap. \`printers.json\` has every run.`,
    "",
    `| Printer | | Median over the pages, ${labels[0]} | The printer's | Smallest difference on a page | Median difference | Largest difference | Pages lower / equal / higher | Every run lower | Every run higher |`,
    "|---|---|---|---|---|---|---|---|---|---|",
    ...Object.entries(comparisons).flatMap(([l, c]) => [line(l, "Print time", c.wall_ms, "ms"), line(l, "Peak PSS", c.peak_pss_mb, "MB")]),
    "",
  ];
}

const count = (l, pred) => rows.filter((r) => pred(r) && r.printers[l].pass).length;
const isPage = (r) => r.what === "page";
const realPages = rows.filter((r) => r.set === "real-world" && isPage(r)).length;
const islandPages = rows.filter((r) => r.set === "islands" && isPage(r)).length;

const md = [
  "# Printers against Chrome",
  "",
  `The same pages printed by ${printers.map((p) => p.version).join(printers.length > 2 ? ", " : " and ").replace(/, ([^,]*)$/, " and $1")}, fixture fonts only, each scored against the committed Chrome references (Chromium 153, pinned) by the metric and pass rule of \`fixtures/README.md\`: shape ≤ ${pct(PASS_WORST_WINDOW)}, colour ≤ ${pct(PASS_COLOUR_WINDOW)}, hue ≤ ${pct(PASS_HUE_WINDOW)}, same page size and count. surimono's output is the same bytes for every printer${values.commit ? ` (commit ${values.commit})` : ""}. Generated by \`fixtures/tools/printers.mjs\`.`,
  "",
  "A cell is the verdict and the shape score (the worst 32×32 window of near-mismatched ink), then any other check that failed. Time is the wall clock of one `fulgur render` process, the median of " +
    `${runs} ${runs === 1 ? "run" : "runs"}; peak PSS is the peak of summed PSS over that process, the median of ${runs} more, sampled every ${MEASURE_INTERVAL_MS} ms (\`bench/tools/measure.py\`, the Grover benchmark's sampler).${printers.length > 1 ? " The printers take turns on each page, round by round after one round that isn't measured, and who starts moves on every round and every page." : ""} "Same pixels" says whether the printers' pages are pixel for pixel the same at 96 dpi, "Same PDF" whether the files are byte for byte the same. An image is there only where the verdict or a score differs: Chrome | ${labels.join(" | ")}.`,
  "",
  "| Printer | Real-world pages that pass | Their floors that pass | Island fixture pages that pass | Islands alone that pass | Cells that read differently from " + `${labels[0]} | Median print time | Median peak PSS | Largest median peak PSS of a page |`,
  "|---|---|---|---|---|---|---|---|---|",
  ...labels.map(
    (l) =>
      `| ${printers.find((p) => p.label === l).version} | ${count(l, (r) => r.set === "real-world" && isPage(r))} of ${realPages} | ${count(l, (r) => r.what === "floor")} of ${realPages} | ${count(l, (r) => r.set === "islands" && isPage(r))} of ${islandPages} | ${count(l, (r) => r.what.startsWith("island-"))} of ${rows.filter((r) => r.what.startsWith("island-")).length} | ${l === labels[0] ? "—" : rows.filter((r) => signature({ ...r.printers[l] }) !== signature({ ...r.printers[labels[0]] })).length} | ${ms(totals[l].wall_ms_median)} | ${mb(totals[l].peak_pss_mb_median)} | ${mb(totals[l].peak_pss_mb_max)} |`,
  ),
  "",
  `Medians are over the ${measuredRows.length} prints every printer finished (pages and floors). Of the ${rows.length} cells, ${rows.filter((r) => r.differs).length} read differently between printers, ${rows.filter((r) => r.pixels_identical).length} are the same pixels and ${rows.filter((r) => r.pdf_identical).length} the same PDF.`,
  "",
  ...comparisonTable(),
  "## Real-world pages",
  "",
  "The floor is the printer on Chrome's own DOM (`reference/dom.html`): what it can reach on that page whatever surimono does.",
  "",
  ...table("real-world", true),
  "",
  "## Island fixture pages",
  "",
  ...table("islands", true),
  "",
  "## Islands alone",
  "",
  "surimono's SVG for one island on a page of exactly its size. Not timed.",
  "",
  ...table("islands", false),
  "",
].join("\n");

const json = {
  generated_by: "fixtures/tools/printers.mjs",
  commit: values.commit ?? null,
  runs,
  pass_rule: { shape: PASS_WORST_WINDOW, colour: PASS_COLOUR_WINDOW, hue: PASS_HUE_WINDOW },
  printers: printers.map(({ label, version, sha256: hash }) => ({ label, version, sha256: hash })),
  surimono_outputs_sha256: results[0].outputs,
  measurement: { runs, sampled_runs: runs, warmup_rounds: 1, sample_interval_ms: MEASURE_INTERVAL_MS, order: "the printers take turns on each page; who starts moves on by one every round and every page (rows[].print_order)" },
  totals,
  against_first: comparisons,
  rows,
};
/** No machine path in what gets committed. */
const relabel = (text) => text.split(`${outRoot}/`).join("printers-out/").split(outRoot).join("printers-out").split(`${REPO}/`).join("").split(REPO).join(".");
fs.rmSync(path.join(outRoot, ".private"), { recursive: true, force: true });
if (failures.length) {
  fs.mkdirSync(path.join(outRoot, ".private"), { recursive: true });
  fs.writeFileSync(path.join(outRoot, ".private", "failures.json"), `${JSON.stringify(failures, null, 2)}\n`);
  console.error(`${failures.length} cell(s) without output; the reasons are in ${path.join(outRoot, ".private", "failures.json")}, which isn't for committing`);
}
fs.writeFileSync(path.join(outRoot, "printers.json"), relabel(`${JSON.stringify(json, null, 2)}\n`));
fs.writeFileSync(path.join(outRoot, "index.md"), relabel(md));
console.log(`\n${relabel(md)}`);
