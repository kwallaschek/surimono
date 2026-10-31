// The verdict of one comparison's score.json, for checkpoint.mjs's index.md.
// A pass is derived the way compat.py derives it, never taken on trust: the
// top-level `pass` must be exactly true, at least one page must have been
// compared, and no check inside the score (a page, text, font) may have
// failed, where a check that is present fails unless its `pass` is exactly true.

/** A share as a percentage with one decimal, or "—" when it isn't a number. */
export const pct = (x) => (typeof x === "number" ? `${(x * 100).toFixed(1)}%` : "—");

const passedCheck = (c) => c !== null && typeof c === "object" && c.pass === true;
const list = (x) => (Array.isArray(x) ? x : []);

/** The checks a score failed, by name, in the order compat.py's failed_checks gives them. */
export function failedChecks(s) {
  const names = [];
  const add = (n) => names.includes(n) || names.push(n);
  if (s.error) add("render error");
  const pages = list(s.pages);
  if (!pages.length && !s.error) add("no pages");
  for (const p of pages) {
    const listed = p !== null && typeof p === "object" ? p.failed : undefined;
    if (Array.isArray(listed)) {
      if (listed.length) listed.forEach((n) => add(String(n)));
      else if (!passedCheck(p)) add("page");
    } else if (listed || !passedCheck(p)) add("page"); // a "failed" that isn't a list fails the page
  }
  for (const c of ["text", "font"]) if (s[c] !== undefined && s[c] !== null && !passedCheck(s[c])) add(c);
  return names;
}

/** Whether a score passes (see the top of this file). */
export const passes = (s) => Boolean(s) && s.pass === true && !s.error && failedChecks(s).length === 0;

/** One failed check with its own number. */
function detail(s, c) {
  const t = s.text !== null && typeof s.text === "object" ? s.text : {};
  const f = s.font !== null && typeof s.font === "object" ? s.font : {};
  switch (c) {
    case "shape":
      return `shape ${pct(s.worst_window)}`;
    case "colour":
      return `colour ${pct(s.worst_colour_window)}`;
    case "hue":
      return `hue ${pct(s.worst_hue_window)}`;
    case "text":
      return `text: ${list(t.missing).length} missing, ${list(t.extra).length} extra`;
    case "font": {
      const parts = [["differ", f.mismatches], ["missing", f.missing], ["extra", f.extra]].filter(([, xs]) => list(xs).length).map(([k, xs]) => `${list(xs).length} ${k}`);
      return `font: ${parts.join(", ") || "differs"}`;
    }
    case "no pages":
      return "no pages compared";
    case "page":
      return "page count";
    default:
      return c;
  }
}

/**
 * The numbers next to a verdict: shape / colour / hue, then "text ok" or
 * "font ok" only when that check's `pass` is exactly true ("differs"
 * otherwise); "—" for a render error.
 */
export function scoresCell(s) {
  if (!s || s.error) return "—";
  const check = (name) => (s[name] === undefined || s[name] === null ? "" : ` / ${name} ${passedCheck(s[name]) ? "ok" : "differs"}`);
  return `${pct(s.worst_window)} / ${pct(s.worst_colour_window)} / ${pct(s.worst_hue_window)}${check("text")}${check("font")}`;
}

/** "pass", or "**FAIL** (…)" with each failed check and its own number (the format compat.py uses). */
export function verdict(s) {
  if (!s) return "not run";
  if (s.error) return `**FAIL** (render error: ${String(s.error).split("\n")[0].slice(0, 100)})`;
  if (passes(s)) return "pass";
  const failed = failedChecks(s);
  // Nothing inside failed, so the producer didn't mark it as passing.
  return `**FAIL** (${(failed.length ? failed : [`pass is ${JSON.stringify(s.pass)}`]).map((c) => detail(s, c)).join("; ")})`;
}
