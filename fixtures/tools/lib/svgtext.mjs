// Text content of an island SVG, for comparing what a chart says rather than
// how it looks. Chrome's SVG holds its strings in <text> elements. surimono
// may print text as vector outlines (fulgur 0.40 would otherwise draw SVG text
// with system fonts) and keeps each original string in an aria-label or a
// <title> on the outline. So a candidate's strings are its <text> contents plus
// its labels, minus the labels Chrome's SVG already has (chart libraries add
// aria-labels and titles of their own, e.g. Vega's axis descriptions).
//
// Strings are compared as multisets after decoding XML entities and removing
// all whitespace, so tspan line breaks and spacing conventions don't matter;
// changed, missing or extra characters do. Every copy counts: a string the
// candidate draws twice (two outlines, or an outline and a <text>) where
// Chrome draws it once is extra.

const NAMED = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: "\u00a0" };

export function decodeXml(s) {
  return s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (m, e) => {
    if (e[0] === "#") return String.fromCodePoint(e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10));
    return NAMED[e] ?? m;
  });
}

/** Decoded, with every whitespace character removed. */
export const normaliseText = (s) => decodeXml(s.replace(/<[^>]*>/g, "")).replace(/\s+/g, "");

/** <title> and <desc> elements with their contents: tooltips, not printed text. */
const NOT_PRINTED = /<(title|desc)\b[^>]*>[\s\S]*?<\/\1\s*>/g;

/** { text: [<text> contents], labels: [aria-label values and <title> contents] }, each sorted, empty strings dropped. */
export function svgTexts(svg) {
  // Self-closing elements have no content; dropped first so a lazy match can't
  // run on from `<text/>` to the next element's closing tag.
  const open = svg.replace(/<(text|title)\b[^>]*\/>/g, "");
  // A <title> inside <text> is a tooltip, not printed: it counts as a label only
  // (as in surimono's outline aria-label, which leaves it out).
  const text = [...open.matchAll(/<text\b[^>]*>([\s\S]*?)<\/text\s*>/g)].map((m) => normaliseText(m[1].replace(NOT_PRINTED, "")));
  const labels = [
    ...[...open.matchAll(/\saria-label\s*=\s*(?:"([^"]*)"|'([^']*)')/g)].map((m) => normaliseText(m[1] ?? m[2])),
    ...[...open.matchAll(/<title\b[^>]*>([\s\S]*?)<\/title\s*>/g)].map((m) => normaliseText(m[1])),
  ];
  const clean = (list) => list.filter(Boolean).sort();
  return { text: clean(text), labels: clean(labels) };
}

/** Multiset difference a - b of two string lists. */
export function multisetMinus(a, b) {
  const counts = new Map();
  for (const s of b) counts.set(s, (counts.get(s) ?? 0) + 1);
  const out = [];
  for (const s of a) {
    const n = counts.get(s) ?? 0;
    if (n > 0) counts.set(s, n - 1);
    else out.push(s);
  }
  return out;
}

/**
 * Compares a candidate SVG's strings with the reference's (svgTexts of
 * Chrome's SVG). The candidate draws its <text> strings plus its labels that
 * Chrome's SVG doesn't have (the outlines' labels). Passes when those equal
 * Chrome's <text> strings as multisets: nothing missing, and nothing extra,
 * copies included.
 */
export function compareTexts(reference, candidateSvg) {
  const cand = svgTexts(candidateSvg);
  const outlined = multisetMinus(cand.labels, reference.labels);
  const drawn = [...cand.text, ...outlined];
  const missing = multisetMinus(reference.text, drawn);
  const extra = multisetMinus(drawn, reference.text);
  return {
    pass: missing.length === 0 && extra.length === 0,
    reference_strings: reference.text.length,
    candidate_text: cand.text.length,
    candidate_outlined: outlined.length,
    missing,
    extra,
  };
}

// ---------------------------------------------------------------- font weight and style
//
// The image checks can't see a wrong weight when the metrics are close (the
// "all text bold" calibration fault passes on some islands), so the declared
// font-weight and font-style of every piece of text are compared exactly.
// Both sides are resolved the same way, from the SVG markup alone: the
// presentation attribute, then the style attribute (font-weight, font-style,
// or the `font` shorthand, which resets both), inherited from ancestors.
// <style> sheets are not read; surimono's outline mode doesn't apply them
// either (docs/COMPAT.md).
//
// Each text is keyed by its own aria-label if it has one, else by its
// normalised content. surimono's outlines replace <text> with a <g> that keeps
// the text's attributes and carries the string (or the author's aria-label)
// as aria-label, so the same key finds it.

const WEIGHTS = { normal: 400, bold: 700, lighter: 100, bolder: 900 };

/** CSS font-weight value -> number, relative to the inherited weight. */
function weightOf(value, inherited) {
  const v = String(value).trim().toLowerCase();
  if (/^\d+$/.test(v)) return Number(v);
  if (v === "bolder") return inherited < 350 ? 400 : inherited < 550 ? 700 : 900;
  if (v === "lighter") return inherited < 550 ? 100 : inherited < 750 ? 400 : 700;
  return WEIGHTS[v] ?? inherited;
}

/** The weight and style one element declares, given what it inherits. */
function declared(tag, parent) {
  const attr = (name) => {
    const m = new RegExp(`\\s${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)')`, "i").exec(tag);
    return m ? (m[1] ?? m[2]) : undefined;
  };
  let { weight, style } = parent;
  if (attr("font-weight") !== undefined) weight = weightOf(decodeXml(attr("font-weight")), parent.weight);
  if (attr("font-style") !== undefined) style = decodeXml(attr("font-style")).trim().toLowerCase();
  const css = decodeXml(attr("style") ?? "");
  for (const decl of css.split(";")) {
    const i = decl.indexOf(":");
    if (i < 0) continue;
    const prop = decl.slice(0, i).trim().toLowerCase();
    const value = decl.slice(i + 1).replace(/!important/i, "").trim();
    if (prop === "font-weight") weight = weightOf(value, parent.weight);
    else if (prop === "font-style") style = value.toLowerCase();
    else if (prop === "font") {
      // The shorthand resets weight and style to normal unless it names them
      // before the size.
      const head = value.split(/\s+/);
      const sizeAt = head.findIndex((t) => /^[\d.]+(px|pt|em|rem|%)?(\/.*)?$/i.test(t) && !/^\d{3}$/.test(t));
      const pre = (sizeAt < 0 ? head : head.slice(0, sizeAt)).map((t) => t.toLowerCase());
      weight = 400;
      style = "normal";
      for (const t of pre) {
        if (/^\d{3}$/.test(t) || t === "bold" || t === "bolder" || t === "lighter") weight = weightOf(t, parent.weight);
        else if (t === "italic" || t === "oblique") style = t;
      }
    }
  }
  return { weight, style };
}

/**
 * [{ key, weight, style }] for every <text> in the SVG, sorted; with
 * `outlines`, also for every outline group (how surimono's gfx replaces a
 * <text>): a <g> with an aria-label whose descendants are only <path> and <g>
 * elements with neither an aria-label nor a role. The group's own role
 * doesn't matter: gfx keeps the role of the <text> it replaces (Vega's text
 * marks have role="graphics-symbol") and adds role="img" when there is none.
 * So a labelled mark, or a group holding one, never qualifies; a labelled
 * group of plain paths does, whatever its role. `key` is the element's
 * aria-label (normalised) or, for a <text> without one, its content.
 */
export function svgTextStyles(svg, { outlines = false } = {}) {
  const out = [];
  const stack = [{ weight: 400, style: "normal", name: "" }];
  const re = /<(\/?)([a-zA-Z][\w:.-]*)((?:\s+[^\s=>]+\s*=\s*(?:"[^"]*"|'[^']*'))*)\s*(\/?)>|<!--[\s\S]*?-->|<!\[CDATA\[[\s\S]*?\]\]>|<\?[\s\S]*?\?>/g;
  let open = null; // a <text> being read: { start, label, weight, style, depth }
  let m;
  while ((m = re.exec(svg))) {
    const [whole, closing, name, , selfClosing] = m;
    if (!name) continue;
    if (closing) {
      const top = stack.pop();
      if (open && name === "text" && stack.length === open.depth) {
        const content = normaliseText(svg.slice(open.start, m.index).replace(/<(title|desc)\b[^>]*>[\s\S]*?<\/\1\s*>/g, ""));
        const key = open.label ?? content;
        if (key) out.push({ key, weight: open.weight, style: open.style });
        open = null;
      }
      if (top?.outline && top.outline.valid) out.push({ key: top.outline.key, weight: top.weight, style: top.style });
      continue;
    }
    const own = declared(whole, stack[stack.length - 1]);
    const label = /\saria-label\s*=\s*(?:"([^"]*)"|'([^']*)')/.exec(whole);
    const labelKey = label ? normaliseText(label[1] ?? label[2]) : undefined;
    // Any element inside an open outline candidate other than an unlabelled
    // <path> or <g> means it isn't an outline group.
    if (!(name === "path" || name === "g") || label || /\srole\s*=/.test(whole)) {
      for (const e of stack) if (e.outline) e.outline.valid = false;
    }
    if (name === "text" && !selfClosing && !open) open = { start: m.index + whole.length, label: labelKey || undefined, ...own, depth: stack.length };
    const outline = outlines && name === "g" && labelKey && !open && !selfClosing ? { key: labelKey, valid: true } : undefined;
    if (!selfClosing) stack.push({ ...own, name, outline });
  }
  return out.sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : a.weight - b.weight || (a.style < b.style ? -1 : a.style > b.style ? 1 : 0)));
}

/**
 * Compares the declared weight and style of every reference <text> with the
 * candidate's texts and outline groups, one to one per key: each reference
 * entry consumes one candidate element with its key and the same weight and
 * style. A reference entry left over is a mismatch (a wrong weight or style)
 * or, when the candidate has fewer elements with that key, missing. A
 * candidate element left over (a second copy of a label, or a key Chrome has
 * no text for) is extra. Passes when nothing is left over on either side.
 */
export function compareStyles(referenceStyles, candidateSvg) {
  const pool = new Map();
  for (const s of svgTextStyles(candidateSvg, { outlines: true })) {
    if (!pool.has(s.key)) pool.set(s.key, []);
    pool.get(s.key).push(`${s.weight} ${s.style}`);
  }
  const byKey = new Map();
  for (const r of referenceStyles) {
    if (!byKey.has(r.key)) byKey.set(r.key, []);
    byKey.get(r.key).push(`${r.weight} ${r.style}`);
  }
  const mismatches = [];
  const missing = [];
  const extra = [];
  let compared = 0;
  for (const [key, wanted] of byKey) {
    const have = [...(pool.get(key) ?? [])];
    const left = [];
    for (const w of wanted) {
      const i = have.indexOf(w);
      if (i >= 0) {
        have.splice(i, 1);
        compared++;
      } else left.push(w);
    }
    // Leftovers pair with the candidate's remaining elements of that key as
    // wrong weights; any beyond those are missing.
    left.forEach((w, i) => {
      if (i < have.length) {
        compared++;
        mismatches.push({ key, chrome: w, surimono: have[i] });
      } else missing.push({ key, chrome: w });
    });
    // Candidate elements of that key beyond Chrome's count.
    for (const h of have.slice(left.length)) extra.push({ key, surimono: h });
  }
  for (const [key, have] of pool) if (!byKey.has(key)) for (const h of have) extra.push({ key, surimono: h });
  return { pass: mismatches.length === 0 && missing.length === 0 && extra.length === 0, compared, mismatches, missing, extra };
}
