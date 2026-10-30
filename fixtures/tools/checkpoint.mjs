#!/usr/bin/env node
// The checkpoint-2 run: every island fixture through surimono (text as
// outlines, the default) and fulgur, compared with the committed Chrome
// references.
//
//   node checkpoint.mjs --out DIR --surimono BIN [fixture ...]
//
// BIN is the surimono CLI (`surimono render <in.html> -o <out.html> --font …`,
// crates/cli), or $SURIMONO_BIN. For each fixture, in DIR/<fixture>/:
//
//   surimono.html             surimono's output for input.html (fixture fonts, default limits)
//   island-<i>.svg            island i cut out of it (the facade's div.surimono-island)
//   island-<i>-chrome/        Chrome prints that SVG alone: surimono's SVG by itself,
//                             the checkpoint-2 gate (shape, colour, hue and text checks)
//   island-<i>-fulgur/        fulgur prints that SVG alone: plus fulgur's SVG printing
//   page/                     fulgur prints surimono.html: the whole document
//
// Each of those has compare.mjs's score.json and side-by-side-1.png (Chrome |
// surimono | diff). surimono runs twice, in two processes; the two outputs
// must be byte-identical (docs/SPEC.md, determinism).
//
// Nothing fails silently: when surimono, fulgur or a comparison dies before
// compare.mjs writes its score, this writes a failing score.json with
// "error" in its place, so compat.py reports a failure, never "not run".
//
// Writes DIR/checkpoint.json and DIR/index.md (a table per fixture with links
// to the images). Regenerate docs/COMPAT.md from it with
//   python3 -I fixtures/tools/compat.py --fixtures DIR --commit <sha>
// Exits 0 when everything ran (even if comparisons fail: the scores are the
// result), 1 when the run itself couldn't start.

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { parseArgs } from "node:util";
import { FIXTURES_DIR, TOOLS_DIR, islandsOf, listFixtures } from "./lib/fixtures.mjs";
import { fulgurPrint, fulgurVersion } from "./lib/fulgur.mjs";
import { PASS_COLOUR_WINDOW, PASS_HUE_WINDOW, PASS_WORST_WINDOW, referenceFiles } from "./lib/compare.mjs";

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: { out: { type: "string" }, surimono: { type: "string" }, kind: { type: "string", default: "islands" } },
});
const bin = values.surimono || process.env.SURIMONO_BIN;
if (!values.out || !bin) {
  console.error("usage: node checkpoint.mjs --out DIR --surimono BIN [fixture ...]   (or set SURIMONO_BIN)");
  process.exit(2);
}
const outRoot = path.resolve(values.out);
fs.mkdirSync(outRoot, { recursive: true });
const FONTS = fs
  .readdirSync(path.join(FIXTURES_DIR, "fonts"))
  .filter((f) => /\.(ttf|otf)$/i.test(f))
  .sort()
  .map((f) => path.join(FIXTURES_DIR, "fonts", f));

const version = spawnSync(bin, ["--version"], { encoding: "utf8" });
if (version.status !== 0) {
  console.error(`${bin} --version failed: ${version.error?.message ?? version.stderr}`);
  process.exit(1);
}
const versions = { surimono: version.stdout.trim(), fulgur: fulgurVersion() };
console.log(`${versions.surimono}; fulgur ${versions.fulgur}`);

const sha256 = (file) => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");

/** What compare.mjs would have written, failing, when the step before it died. */
function errorScore(dir, subject, error) {
  fs.mkdirSync(dir, { recursive: true });
  const score = { subject, pass: false, error: String(error).trim(), pages: [] };
  fs.writeFileSync(path.join(dir, "score.json"), `${JSON.stringify(score, null, 2)}\n`);
  return score;
}

/** Runs compare.mjs (report-only: a failing score is a result, not an error) and returns its score. */
function compare(args, dir, subject) {
  fs.rmSync(path.join(dir, "score.json"), { force: true });
  const r = spawnSync(process.execPath, [path.join(TOOLS_DIR, "compare.mjs"), "--report-only", "--label", "surimono", ...args, dir], {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  process.stdout.write(r.stdout);
  const file = path.join(dir, "score.json");
  if (r.status !== 0 || !fs.existsSync(file)) {
    return errorScore(dir, subject, `compare.mjs exited ${r.status}: ${`${r.stderr}${r.stdout}`.split("\n").slice(-15).join("\n")}`);
  }
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

/** surimono render; returns { ok, ms, error }. */
function render(input, output) {
  const args = ["render", input, "-o", output, ...FONTS.flatMap((f) => ["--font", f])];
  const t0 = process.hrtime.bigint();
  const r = spawnSync(bin, args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  const ms = Number(process.hrtime.bigint() - t0) / 1e6;
  if (r.status !== 0) return { ok: false, ms, error: `surimono exited ${r.status ?? r.signal}: ${`${r.stderr}${r.stdout}`.trim().split("\n").slice(0, 15).join("\n")}` };
  return { ok: true, ms };
}

const rows = [];
for (const fixture of listFixtures(values.kind, positionals)) {
  const dir = path.join(outRoot, fixture.name);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const n = referenceFiles(fixture.dir).islands.length;
  const row = { fixture: fixture.name, islands: [], page: null };
  console.log(`\n== ${fixture.name} (${n} island${n === 1 ? "" : "s"})`);

  const html = path.join(dir, "surimono.html");
  const first = render(path.join(fixture.dir, "input.html"), html);
  row.render_ms = Math.round(first.ms);
  let islands = [];
  let failure = first.ok ? null : first.error;
  if (first.ok) {
    const again = render(path.join(fixture.dir, "input.html"), path.join(dir, "surimono-2.html"));
    row.deterministic = again.ok && sha256(html) === sha256(path.join(dir, "surimono-2.html"));
    if (!row.deterministic) row.determinism_note = again.ok ? "the second render's bytes differ" : again.error;
    row.sha256 = sha256(html);
    islands = islandsOf(fs.readFileSync(html, "utf8"));
    if (islands.length !== n) failure = `surimono's output has ${islands.length} islands, the fixture has ${n}`;
  }
  for (let i = 0; i < n; i++) {
    const chromeDir = path.join(dir, `island-${i}-chrome`);
    const fulgurDir = path.join(dir, `island-${i}-fulgur`);
    const subject = (printer) => `${fixture.name} island ${i} (printed by ${printer})`;
    const why = failure ?? islands[i]?.error;
    if (why) {
      row.islands.push({ chrome: errorScore(chromeDir, subject("chrome"), why), fulgur: errorScore(fulgurDir, subject("fulgur"), why) });
      continue;
    }
    const svg = path.join(dir, `island-${i}.svg`);
    fs.writeFileSync(svg, islands[i].svg);
    row.islands.push({
      chrome: compare(["--island", String(i), "--printer", "chrome", fixture.dir, svg], chromeDir, subject("chrome")),
      fulgur: compare(["--island", String(i), "--printer", "fulgur", fixture.dir, svg], fulgurDir, subject("fulgur")),
    });
  }
  const pageDir = path.join(dir, "page");
  if (failure) {
    row.page = errorScore(pageDir, fixture.name, failure);
  } else {
    try {
      fs.mkdirSync(pageDir, { recursive: true });
      fulgurPrint(html, path.join(pageDir, "surimono-fulgur.pdf"), pageDir);
      row.page = compare([fixture.dir, path.join(pageDir, "surimono-fulgur.pdf")], pageDir, fixture.name);
    } catch (e) {
      row.page = errorScore(pageDir, fixture.name, e.message);
    }
  }
  rows.push(row);
}

// ---------------------------------------------------------------- report

const pct = (x) => (typeof x === "number" ? `${(x * 100).toFixed(1)}%` : "—");
const rel = (file) => path.relative(outRoot, file).split(path.sep).join("/");
/** pass/FAIL with each failed check and its own number (the format compat.py uses). */
function verdict(s) {
  if (!s) return "not run";
  if (s.error) return `**FAIL** (render error: ${s.error.split("\n")[0].slice(0, 100)})`;
  const failed = [...new Set((s.pages ?? []).flatMap((p) => p.failed ?? []))];
  if (s.text && !s.text.pass) failed.push("text");
  if (s.pass) return "pass";
  const detail = failed.map((c) =>
    c === "shape" ? `shape ${pct(s.worst_window)}` : c === "colour" ? `colour ${pct(s.worst_colour_window)}` : c === "hue" ? `hue ${pct(s.worst_hue_window)}` : c === "text" ? `text: ${(s.text.missing ?? []).length} missing, ${(s.text.extra ?? []).length} extra` : c,
  );
  return `**FAIL** (${detail.join("; ") || "page count"})`;
}
const scoresCell = (s) => (s && !s.error ? `${pct(s.worst_window)} / ${pct(s.worst_colour_window)} / ${pct(s.worst_hue_window)}${s.text ? ` / text ${s.text.pass ? "ok" : "differs"}` : ""}` : "—");
const image = (dirName, label) => {
  const f = path.join(outRoot, dirName, "side-by-side-1.png");
  return fs.existsSync(f) ? `[${label}](${rel(f)})` : "—";
};

const all = rows.flatMap((r) => [...r.islands.flatMap((x) => [x.chrome, x.fulgur]), r.page]);
const gate = rows.flatMap((r) => r.islands.map((x) => x.chrome));
const md = [
  "# Checkpoint 2: surimono against Chrome",
  "",
  `${versions.surimono}, text as outlines (the default), fixture fonts only; fulgur ${versions.fulgur}. Chrome references: \`fixtures/islands/*/reference/\` (Chromium 153, pinned). Generated by \`fixtures/tools/checkpoint.mjs\`.`,
  "",
  `**Gate (each island's SVG printed by Chrome): ${gate.filter((s) => s.pass && !s.error).length} of ${gate.length} pass.** All comparisons: ${all.filter((s) => s.pass && !s.error).length} of ${all.length} pass. Byte-identical across two processes: ${rows.filter((r) => r.deterministic).length} of the ${rows.filter((r) => r.deterministic !== undefined).length} fixtures that rendered (${rows.length} in all).`,
  "",
  `A comparison passes when shape (worst 32×32 window of near-mismatched ink) ≤ ${pct(PASS_WORST_WINDOW)}, colour (solid areas) ≤ ${pct(PASS_COLOUR_WINDOW)}, hue (strokes) ≤ ${pct(PASS_HUE_WINDOW)}, the page size matches and, for islands, every Chrome string is there (\`fixtures/README.md\`). Scores are shape / colour / hue. Each image is Chrome | surimono | diff.`,
  "",
  "| Fixture | Island | Printed by Chrome (gate) | Scores | Printed by fulgur | Scores | Images |",
  "|---|---|---|---|---|---|---|",
  ...rows.flatMap((r) =>
    r.islands.map(
      (x, i) =>
        `| ${r.fixture} | ${i} | ${verdict(x.chrome)} | ${scoresCell(x.chrome)} | ${verdict(x.fulgur)} | ${scoresCell(x.fulgur)} | ${image(`${r.fixture}/island-${i}-chrome`, "Chrome-printed")}, ${image(`${r.fixture}/island-${i}-fulgur`, "fulgur-printed")} |`,
    ),
  ),
  "",
  "## Whole pages (surimono, then fulgur)",
  "",
  "| Fixture | Page | Scores | Image | surimono render (first run) | Deterministic |",
  "|---|---|---|---|---|---|",
  ...rows.map(
    (r) =>
      `| ${r.fixture} | ${verdict(r.page)} | ${scoresCell(r.page)} | ${image(`${r.fixture}/page`, "page")} | ${r.render_ms} ms | ${r.deterministic === undefined ? "—" : r.deterministic ? "yes" : `**no** (${r.determinism_note})`} |`,
  ),
  "",
  "fulgur's own floor (printing Chrome's SVGs, `fixtures/README.md`, baseline) is at most 4.5% on a page and 3.9% on an island, so a fulgur-printed score above that comes from surimono's SVG.",
  "",
].join("\n");
fs.writeFileSync(path.join(outRoot, "checkpoint.json"), `${JSON.stringify({ versions, rows }, null, 2)}\n`);
fs.writeFileSync(path.join(outRoot, "index.md"), md);
console.log(`\n${md}`);
