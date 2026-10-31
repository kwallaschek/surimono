#!/usr/bin/env node
// The checkpoint-2 run: every island fixture through surimono (text as
// outlines, the default) and fulgur, compared with the committed Chrome
// references.
//
//   node checkpoint.mjs --out DIR --surimono BIN [--commit SHA] [--baseline BDIR] [fixture ...]
//
// BIN is the surimono CLI (`surimono render <in.html> -o <out.html> --font …`,
// crates/cli), or $SURIMONO_BIN; --commit (or $SURIMONO_COMMIT) names the
// commit it was built from. DIR must be outside fixtures/. BDIR, if given, is
// a `node reference.mjs --out BDIR && node baseline.mjs --out BDIR` directory:
// fulgur printing Chrome's own SVGs, whose scores are the floor shown next to
// surimono's fulgur-printed ones. For each fixture, in DIR/<fixture>/:
//
//   surimono.html             surimono's output for input.html (fixture fonts, default limits)
//   island-<i>.svg            island i cut out of it (the facade's div.surimono-island)
//   island-<i>-chrome/        Chrome prints that SVG alone: surimono's SVG by itself,
//                             the checkpoint-2 gate (shape, colour, hue, text and font checks)
//   island-<i>-fulgur/        fulgur prints that SVG alone: plus fulgur's SVG printing
//   page/                     fulgur prints surimono.html: the whole document
//
// Each of those has compare.mjs's score.json and side-by-side-1.png (Chrome |
// surimono | diff), stamped with the CLI that produced it ("producer": path,
// version, sha256, commit). surimono runs twice, in two processes with a
// different time zone, locale and working directory; the two outputs must be
// byte-identical (docs/SPEC.md, determinism), recorded in determinism.json.
//
// Nothing fails silently: when surimono, fulgur or a comparison dies before
// compare.mjs writes its score, this writes a failing score.json with
// "error" in its place, so compat.py reports a failure, never "not run".
//
// Writes DIR/checkpoint.json and DIR/index.md (a table per fixture with links
// to the images). Regenerate docs/COMPAT.md from it with
//   python3 -I fixtures/tools/compat.py --fixtures DIR --commit <sha>
// Exits 0 when everything ran (even if comparisons fail: the scores are the
// result), 1 when a fixture's two renders differ (determinism is a hard rule)
// or the run couldn't start, 2 on usage errors.

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { parseArgs } from "node:util";
import os from "node:os";
import { FIXTURES_DIR, TOOLS_DIR, islandsOf, listFixtures } from "./lib/fixtures.mjs";
import { fulgurPrint, fulgurVersion } from "./lib/fulgur.mjs";
import { PASS_COLOUR_WINDOW, PASS_HUE_WINDOW, PASS_WORST_WINDOW, referenceFiles } from "./lib/compare.mjs";
import { passes, pct, verdict } from "./lib/report.mjs";

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    out: { type: "string" },
    surimono: { type: "string" },
    commit: { type: "string" },
    baseline: { type: "string" },
    kind: { type: "string", default: "islands" },
  },
});
const bin = values.surimono || process.env.SURIMONO_BIN;
if (!values.out || !bin) {
  console.error("usage: node checkpoint.mjs --out DIR --surimono BIN [--commit SHA] [--baseline BDIR] [fixture ...]   (or set SURIMONO_BIN)");
  process.exit(2);
}
const outRoot = path.resolve(values.out);
const inFixtures = path.relative(FIXTURES_DIR, outRoot);
if (!inFixtures.startsWith("..") && !path.isAbsolute(inFixtures)) {
  // Outputs inside fixtures/ could be committed or picked up as references.
  console.error(`--out ${outRoot} is inside fixtures/; write results elsewhere`);
  process.exit(2);
}
fs.mkdirSync(outRoot, { recursive: true });
const FONTS = fs
  .readdirSync(path.join(FIXTURES_DIR, "fonts"))
  .filter((f) => /\.(ttf|otf)$/i.test(f))
  .sort()
  .map((f) => path.join(FIXTURES_DIR, "fonts", f));
const REPO = path.resolve(FIXTURES_DIR, "..");

const sha256 = (file) => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");

/** The CLI's absolute path ($PATH lookup for a bare name). */
function resolveBin(name) {
  if (name.includes("/")) return path.resolve(name);
  for (const dir of (process.env.PATH || "").split(path.delimiter)) {
    const f = path.join(dir, name);
    if (fs.existsSync(f)) return f;
  }
  return name;
}
const cli = resolveBin(bin);
const version = spawnSync(cli, ["--version"], { encoding: "utf8" });
if (version.status !== 0) {
  console.error(`${cli} --version failed: ${version.error?.message ?? version.stderr}`);
  process.exit(1);
}
/** Which CLI produced every result of this run; compat.py refuses a stand-in's. */
const producer = {
  cli,
  version: version.stdout.trim(),
  sha256: fs.existsSync(cli) ? sha256(cli) : null,
  commit: values.commit || process.env.SURIMONO_COMMIT || null,
};
let fulgur;
try {
  fulgur = fulgurVersion();
} catch (e) {
  fulgur = `not found (${e.message.split("\n")[0]})`;
}
const versions = { surimono: producer.version, fulgur };
console.log(`${versions.surimono}; fulgur ${versions.fulgur}`);

/** Adds the producer to a score and writes it as the comparison's score.json. */
function stamp(dir, score) {
  const stamped = { ...score, producer };
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "score.json"), `${JSON.stringify(stamped, null, 2)}\n`);
  return stamped;
}

/** What compare.mjs would have written, failing, when the step before it died. */
function errorScore(dir, subject, error) {
  return stamp(dir, { subject, pass: false, error: String(error).trim(), pages: [] });
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
  return stamp(dir, JSON.parse(fs.readFileSync(file, "utf8")));
}

/** fulgur's floor per fixture from a baseline.mjs run: { page, islands: [..] } worst windows. */
function loadBaseline(dir) {
  if (!dir) return null;
  const file = path.join(path.resolve(dir), "baseline.json");
  if (!fs.existsSync(file)) {
    console.error(`--baseline: no ${file} (run reference.mjs --out and baseline.mjs --out there first)`);
    process.exit(2);
  }
  const data = JSON.parse(fs.readFileSync(file, "utf8"));
  return { fulgur: data.fulgur, rows: Object.fromEntries(data.rows.map((r) => [r.fixture, { page: r.page?.worst_window, islands: r.islands.map((x) => x?.worst_window) }])) };
}
const baseline = loadBaseline(values.baseline);

/**
 * The two environments surimono renders in: output must not depend on the
 * host's time zone, locale or working directory.
 */
/**
 * The second run's locale: the first of these that `locale -a` lists. The
 * decimal-comma locales come first, so a leak through printf or strtod
 * (1.5 printed or read as 1,5) shows too; ja_JP and en_US share C's ".". If
 * none is installed, de_DE.UTF-8 is set anyway: that still catches code
 * reading LANG or LC_ALL, but not libc's locale behaviour, and the report says so.
 */
const SECOND_LOCALES = ["de_DE.UTF-8", "fr_FR.UTF-8", "ja_JP.UTF-8", "en_US.UTF-8"];
function secondLocale() {
  const r = spawnSync("locale", ["-a"], { encoding: "utf8" });
  const have = new Set((r.stdout ?? "").split("\n").map((l) => l.trim().toLowerCase().replace("utf-8", "utf8")));
  for (const want of SECOND_LOCALES) {
    if (have.has(want.toLowerCase().replace("utf-8", "utf8"))) return { locale: want, installed: true };
  }
  return { locale: SECOND_LOCALES[0], installed: false };
}
const second = secondLocale();
if (!second.installed) console.log(`note: no ${SECOND_LOCALES.map((l) => l.split(".")[0]).join(", ")} UTF-8 locale is installed (locale -a); the second render sets LANG=${second.locale} anyway, which catches code that reads LANG but not libc's locale behaviour`);
const RUNS = [
  { TZ: "UTC", LANG: "C.UTF-8", LC_ALL: "C.UTF-8", cwd: REPO, locale_installed: true },
  { TZ: "Asia/Tokyo", LANG: second.locale, LC_ALL: second.locale, cwd: null, locale_installed: second.installed },
];

/** Blocks until the next whole second plus 100 ms, so two renders never share a second of the wall clock. */
function nextSecond() {
  const wait = 1100 - (Date.now() % 1000);
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, wait);
}

/** surimono render in environment `run`; returns { ok, ms, error }. */
function render(input, output, run) {
  const args = ["render", input, "-o", output, ...FONTS.flatMap((f) => ["--font", f])];
  const cwd = run.cwd ?? fs.mkdtempSync(path.join(os.tmpdir(), "surimono-cwd-"));
  const env = { ...process.env, TZ: run.TZ, LANG: run.LANG, LC_ALL: run.LC_ALL };
  const t0 = process.hrtime.bigint();
  const r = spawnSync(cli, args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024, env, cwd });
  const ms = Number(process.hrtime.bigint() - t0) / 1e6;
  if (!run.cwd) fs.rmSync(cwd, { recursive: true, force: true });
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
  const first = render(path.join(fixture.dir, "input.html"), html, RUNS[0]);
  row.render_ms = Math.round(first.ms);
  let islands = [];
  let failure = first.ok ? null : first.error;
  if (first.ok) {
    // A different whole second, so output that leaks the wall clock (at one-second
    // resolution or finer) can't match by chance.
    nextSecond();
    const again = render(path.join(fixture.dir, "input.html"), path.join(dir, "surimono-2.html"), RUNS[1]);
    row.deterministic = again.ok && sha256(html) === sha256(path.join(dir, "surimono-2.html"));
    if (!row.deterministic) row.determinism_note = again.ok ? "the second render's bytes differ" : again.error;
    row.sha256 = sha256(html);
    fs.writeFileSync(
      path.join(dir, "determinism.json"),
      `${JSON.stringify({ subject: fixture.name, deterministic: row.deterministic, note: row.determinism_note ?? null, sha256: row.sha256, runs: RUNS.map((r) => ({ ...r, cwd: r.cwd ?? "a fresh temporary directory" })), producer }, null, 2)}\n`,
    );
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

const rel = (file) => path.relative(outRoot, file).split(path.sep).join("/");
const floor = (fixture, key, i) => {
  if (!baseline) return "—";
  const b = baseline.rows[fixture];
  return b ? pct(key === "page" ? b.page : b.islands[i]) : "not in the baseline";
};
const scoresCell = (s) => (s && !s.error ? `${pct(s.worst_window)} / ${pct(s.worst_colour_window)} / ${pct(s.worst_hue_window)}${s.text ? ` / text ${s.text.pass ? "ok" : "differs"}` : ""}${s.font ? ` / font ${s.font.pass ? "ok" : "differs"}` : ""}` : "—");
const image = (dirName, label) => {
  const f = path.join(outRoot, dirName, "side-by-side-1.png");
  return fs.existsSync(f) ? `[${label}](${rel(f)})` : "—";
};

function floorNote() {
  if (!baseline) return "fulgur's own floor (fulgur printing Chrome's SVGs) wasn't measured in this run; pass `--baseline BDIR` from `baseline.mjs` to show it.";
  const pages = Object.values(baseline.rows).map((b) => b.page).filter((x) => typeof x === "number");
  const isl = Object.values(baseline.rows).flatMap((b) => b.islands).filter((x) => typeof x === "number");
  const most = (xs) => (xs.length ? pct(Math.max(...xs)) : "not measured (no scores in the baseline)");
  return `fulgur's own floor, measured by \`baseline.mjs\` (fulgur ${baseline.fulgur} printing Chrome's SVGs): at most ${most(pages)} on a page and ${most(isl)} on an island. A fulgur-printed score above its floor comes from surimono's SVG.`;
}
const all = rows.flatMap((r) => [...r.islands.flatMap((x) => [x.chrome, x.fulgur]), r.page]);
const gate = rows.flatMap((r) => r.islands.map((x) => x.chrome));
const md = [
  "# Checkpoint 2: surimono against Chrome",
  "",
  `${versions.surimono} (\`${producer.cli}\`, sha256 ${producer.sha256?.slice(0, 12) ?? "?"}${producer.commit ? `, commit ${producer.commit}` : ""}), text as outlines (the default), fixture fonts only; fulgur ${versions.fulgur}. The second render runs in a later second of the wall clock with TZ=${RUNS[1].TZ}, ${RUNS[1].LANG}${RUNS[1].locale_installed ? "" : " (not installed here: only the environment variables change)"} and another working directory. Chrome references: \`fixtures/islands/*/reference/\` (Chromium 153, pinned). Generated by \`fixtures/tools/checkpoint.mjs\`.`,
  "",
  `**Gate (each island's SVG printed by Chrome): ${gate.filter(passes).length} of ${gate.length} pass.** All comparisons: ${all.filter(passes).length} of ${all.length} pass. Byte-identical across two processes: ${rows.filter((r) => r.deterministic).length} of the ${rows.filter((r) => r.deterministic !== undefined).length} fixtures that rendered (${rows.length} in all).`,
  "",
  `A comparison passes when shape (worst 32×32 window of near-mismatched ink) ≤ ${pct(PASS_WORST_WINDOW)}, colour (solid areas) ≤ ${pct(PASS_COLOUR_WINDOW)}, hue (strokes) ≤ ${pct(PASS_HUE_WINDOW)}, the page size matches and, for islands, every Chrome string is there with the same declared font weight and style (\`fixtures/README.md\`). Scores are shape / colour / hue. Each image is Chrome | surimono | diff.`,
  "",
  "| Fixture | Island | Printed by Chrome (gate) | Scores | Printed by fulgur | Scores | fulgur's floor | Images |",
  "|---|---|---|---|---|---|---|---|",
  ...rows.flatMap((r) =>
    r.islands.map(
      (x, i) =>
        `| ${r.fixture} | ${i} | ${verdict(x.chrome)} | ${scoresCell(x.chrome)} | ${verdict(x.fulgur)} | ${scoresCell(x.fulgur)} | ${floor(r.fixture, "island", i)} | ${image(`${r.fixture}/island-${i}-chrome`, "Chrome-printed")}, ${image(`${r.fixture}/island-${i}-fulgur`, "fulgur-printed")} |`,
    ),
  ),
  "",
  "## Whole pages (surimono, then fulgur)",
  "",
  "| Fixture | Page | Scores | fulgur's floor | Image | surimono render (first run) | Byte-identical (two processes) |",
  "|---|---|---|---|---|---|---|",
  ...rows.map(
    (r) =>
      `| ${r.fixture} | ${verdict(r.page)} | ${scoresCell(r.page)} | ${floor(r.fixture, "page")} | ${image(`${r.fixture}/page`, "page")} | ${r.render_ms} ms | ${r.deterministic === undefined ? "—" : r.deterministic ? "yes" : `**no** (${r.determinism_note})`} |`,
  ),
  "",
  floorNote(),
  "",
].join("\n");
fs.writeFileSync(path.join(outRoot, "checkpoint.json"), `${JSON.stringify({ versions, producer, rows }, null, 2)}\n`);
fs.writeFileSync(path.join(outRoot, "index.md"), md);
console.log(`\n${md}`);
const differ = rows.filter((r) => r.deterministic === false).map((r) => r.fixture);
if (differ.length) {
  console.error(`\nnot byte-identical across two processes: ${differ.join(", ")}`);
  process.exit(1);
}
