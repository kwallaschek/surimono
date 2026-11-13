// Prints HTML with fulgur the way the fixtures need: the fixture fonts passed
// with --font, and FONTCONFIG_FILE pointing at a config that only knows
// fixtures/fonts, so no system font can reach the PDF (fulgur resolves SVG
// text through fontconfig).

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { FIXTURES_DIR } from "./fixtures.mjs";

const FONTS_DIR = path.join(FIXTURES_DIR, "fonts");

export function fulgurBinary() {
  return process.env.FULGUR || "fulgur";
}

/**
 * Which fulgur every later print uses, for a tool's --fulgur option. It is set in
 * the environment, so the tools this one starts (compare.mjs) print with it too.
 */
export function useFulgur(bin) {
  if (bin) process.env.FULGUR = bin.includes("/") ? path.resolve(bin) : bin;
}

export function fulgurVersion() {
  const r = spawnSync(fulgurBinary(), ["--version"], { encoding: "utf8" });
  if (r.status !== 0) throw new Error(`fulgur not found (set FULGUR or: cargo install fulgur-cli --version =0.40.0 --locked)`);
  return r.stdout.trim().replace(/^fulgur\s+/, "");
}

function fontconfig(workDir) {
  const conf = path.join(workDir, "fonts.conf");
  if (!fs.existsSync(conf)) {
    fs.mkdirSync(workDir, { recursive: true });
    fs.writeFileSync(
      conf,
      `<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>${FONTS_DIR}</dir>
  <cachedir>${path.join(workDir, "fontconfig-cache")}</cachedir>
</fontconfig>
`,
    );
  }
  return conf;
}

/** The fixture fonts and `extraFonts` as --font arguments. */
function fontArgs(extraFonts) {
  return fs
    .readdirSync(FONTS_DIR)
    .filter((f) => /\.(ttf|otf)$/i.test(f))
    .sort()
    .flatMap((f) => ["--font", path.join(FONTS_DIR, f)])
    .concat(extraFonts.flatMap((f) => ["--font", f]));
}

/**
 * The one command every print and every measurement of a print runs: { bin, args, env }.
 * `bin` is the fulgur to run when a tool compares several; otherwise fulgurBinary().
 */
export function fulgurCommand(htmlPath, pdfPath, workDir, extraFonts = [], bin = fulgurBinary()) {
  return {
    bin,
    args: ["render", ...fontArgs(extraFonts), "--creation-date", "2026-01-01", "-o", pdfPath, htmlPath],
    env: { ...process.env, FONTCONFIG_FILE: fontconfig(workDir) },
  };
}

/** A printer that hasn't finished a fixture page by then hangs. */
export const PRINT_TIMEOUT_MS = 120_000;

/** PRINT_TIMEOUT_MS, or $FULGUR_TIMEOUT_MS so a test reaches the timeout in less than two minutes. */
export function printTimeoutMs() {
  const set = Number(process.env.FULGUR_TIMEOUT_MS);
  return Number.isFinite(set) && set > 0 ? set : PRINT_TIMEOUT_MS;
}

/**
 * fulgur render <html> -o <pdf> with only the fixture fonts, and `extraFonts` for a page
 * with its own: fulgur 0.40.0 doesn't load @font-face URLs. Throws with fulgur's output on
 * failure; the error's `printer` says how it ended ({ status, signal, timed_out }).
 * `printer` is the binary when a tool prints with several.
 */
export function fulgurPrint(htmlPath, pdfPath, workDir, extraFonts = [], printer = fulgurBinary()) {
  const { bin, args, env } = fulgurCommand(htmlPath, pdfPath, workDir, extraFonts, printer);
  const timeout = printTimeoutMs();
  const r = spawnSync(bin, args, { encoding: "utf8", env, timeout, killSignal: "SIGKILL" });
  if (r.status !== 0) {
    const timedOut = r.error?.code === "ETIMEDOUT";
    const how = timedOut ? `didn't finish in ${timeout / 1000} s` : r.signal ? `was killed by ${r.signal}` : `exited ${r.status}`;
    const e = new Error(`fulgur ${how} on ${htmlPath}:\n${`${r.stdout ?? ""}${r.stderr ?? ""}`.split("\n").slice(0, 30).join("\n")}`);
    e.printer = { status: r.status, signal: r.signal, timed_out: timedOut };
    throw e;
  }
}

/** A printed PDF's size and hash: two printers that write the same bytes printed the same. */
export function pdfIdentity(pdfPath) {
  const data = fs.readFileSync(pdfPath);
  return { bytes: data.length, sha256: crypto.createHash("sha256").update(data).digest("hex") };
}

const MEASURE = path.resolve(FIXTURES_DIR, "..", "bench", "tools", "measure.py");
/** A print lasts about 50 ms: the benchmark's 10 ms sampler would see it a handful of times. */
export const MEASURE_INTERVAL_MS = 1;

/** measure.py's answer: its JSON, or { error, ... } when it or a run failed. */
function runMeasure(args, env) {
  const r = spawnSync("python3", ["-I", MEASURE, "--interval-ms", String(MEASURE_INTERVAL_MS), "--timeout", String(printTimeoutMs() / 1000), ...args], { encoding: "utf8", env, maxBuffer: 1 << 26 });
  let out;
  try {
    out = JSON.parse(r.stdout);
  } catch {
    return { error: `measure.py exited ${r.status}: ${`${r.stderr}`.split("\n").slice(-5).join("\n")}` };
  }
  if (r.status !== 0) return { error: out.error ?? `measure.py exited ${r.status}`, label: out.label, status: out.status ?? null, signal: out.signal ?? null, timed_out: out.timed_out ?? false };
  return out;
}

/**
 * Time and peak memory of the print fulgurPrint makes, by bench/tools/measure.py:
 * wall clock from spawn to exit (median of `runs` unsampled runs) and the peak
 * of summed PSS over the process tree (`runs` sampled runs), as the Grover
 * benchmark measures a cold run, but sampled every MEASURE_INTERVAL_MS. Returns
 * its statistics with the single numbers behind them (`wall_ms_runs`,
 * `peak_pss_mb_runs`, ...), or { error } when a run failed: a number is measured
 * or absent, never estimated.
 */
export function fulgurMeasure(htmlPath, pdfPath, workDir, extraFonts = [], runs = 5) {
  const { bin, args, env } = fulgurCommand(htmlPath, pdfPath, workDir, extraFonts);
  const out = runMeasure(["--runs", String(runs), "--mem-runs", String(runs), "--", bin, ...args], env);
  if (out.error) return out;
  const { runs: _single, ...summary } = out;
  return { runs, ...summary };
}

/**
 * The same print by several fulgurs, measured in turns so that none of them has the
 * machine at another moment than the others: a round prints once with each, and who
 * starts moves on by one every round, from `offset`, after one round that isn't
 * measured (measure.py --commands).
 * `printers` is [{ label, bin }]. Returns { prints: { label: statistics as
 * fulgurMeasure's }, timed_order, sampled_order }, or { error, label } when a run failed.
 */
export function fulgurMeasureTurns(printers, htmlPath, workDir, extraFonts = [], runs = 5, offset = 0) {
  fs.mkdirSync(workDir, { recursive: true });
  let env;
  const commands = printers.map(({ label, bin }) => {
    const c = fulgurCommand(htmlPath, path.join(workDir, `measure-${label}.pdf`), workDir, extraFonts, bin);
    env = c.env;
    return { label, cmd: [c.bin, ...c.args] };
  });
  const file = path.join(workDir, "measure-commands.json");
  fs.writeFileSync(file, JSON.stringify(commands));
  try {
    const out = runMeasure(["--runs", String(runs), "--mem-runs", String(runs), "--offset", String(offset), "--warmup", "1", "--commands", file], env);
    if (out.error) return out;
    return { prints: Object.fromEntries(Object.entries(out.commands).map(([label, summary]) => [label, { runs, ...summary }])), warmup_rounds: out.warmup_rounds, timed_order: out.timed_order, sampled_order: out.sampled_order };
  } finally {
    fs.rmSync(file, { force: true });
    for (const { label } of printers) fs.rmSync(path.join(workDir, `measure-${label}.pdf`), { force: true });
  }
}
