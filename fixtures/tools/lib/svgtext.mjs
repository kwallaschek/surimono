// Text content of an island SVG, for comparing what a chart says rather than
// how it looks. Chrome's SVG holds its strings in <text> elements. surimono
// may print text as vector outlines (fulgur 0.40 would otherwise draw SVG text
// with system fonts) and keeps each original string in an aria-label or a
// <title> on the outline. So a candidate's strings are its <text> contents plus
// its labels, minus the labels Chrome's SVG already has (chart libraries add
// aria-labels and titles of their own, e.g. Vega's axis descriptions).
//
// gfx also stamps each outline group with the string it outlined and the
// faces that drew it (data-surimono-text, data-surimono-face). Where those
// stamps are present they are read instead of the label: an outline that
// keeps the author's own aria-label (Vega-Lite's text marks) still shows its
// drawn string, and the font check can see the faces actually drawn. Without
// them, the label rules above apply.
//
// Strings are compared as multisets after decoding XML entities and removing
// all whitespace, so tspan line breaks and spacing conventions don't matter;
// changed, missing or extra characters do. Every copy counts: a string the
// candidate draws twice (two outlines, or an outline and a <text>) where
// Chrome draws it once is extra.

import fs from "node:fs";

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

/** An attribute's raw value in a start tag's attribute text, or undefined. */
function attrOf(attrs, name) {
  const m = new RegExp(`\\s${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)')`).exec(attrs);
  return m ? (m[1] ?? m[2]) : undefined;
}

/**
 * gfx's stamped outline groups: [{ text, label }] for every <g> with a
 * data-surimono-text attribute, normalised like svgTexts (text is the string
 * gfx outlined; label the group's aria-label, the author's or gfx's own, or
 * undefined).
 */
export function stampedOutlines(svg) {
  const tags = svg.replace(/<!--[\s\S]*?-->/g, "").matchAll(/<g\b((?:\s+[^\s=>]+\s*=\s*(?:"[^"]*"|'[^']*'))*)\s*\/?>/g);
  const out = [];
  for (const [, attrs] of tags) {
    const text = attrOf(attrs, "data-surimono-text");
    if (text === undefined) continue;
    const label = attrOf(attrs, "aria-label");
    out.push({ text: normaliseText(text), label: label === undefined ? undefined : normaliseText(label) });
  }
  return out;
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
 * Chrome's SVG). The candidate draws its <text> strings, the strings its
 * stamped outline groups record (data-surimono-text; their aria-labels are
 * set aside), and its other labels that Chrome's SVG doesn't have (the labels
 * of unstamped outlines). Passes when those equal Chrome's <text> strings as
 * multisets: nothing missing, and nothing extra, copies included.
 */
export function compareTexts(reference, candidateSvg) {
  const cand = svgTexts(candidateSvg);
  const stamped = stampedOutlines(candidateSvg);
  const labels = multisetMinus(cand.labels, stamped.flatMap((x) => (x.label ? [x.label] : [])));
  const outlined = multisetMinus(labels, reference.labels);
  const fromStamps = stamped.map((x) => x.text).filter(Boolean);
  const drawn = [...cand.text, ...fromStamps, ...outlined];
  const missing = multisetMinus(reference.text, drawn);
  const extra = multisetMinus(drawn, reference.text);
  return {
    pass: missing.length === 0 && extra.length === 0,
    reference_strings: reference.text.length,
    candidate_text: cand.text.length,
    candidate_outlined: fromStamps.length + outlined.length,
    candidate_stamped: fromStamps.length,
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
 * <text>). A <g> with gfx's data-surimono-text stamp is one, whatever it
 * holds (gfx copies a text's <title>, <desc> and <a> into it), and its entry
 * also has `faces`, the data-surimono-face list. Without a stamp, an outline
 * group is a <g> with an aria-label whose descendants are only <path> and <g>
 * elements with neither an aria-label nor a role. The group's own role
 * doesn't matter: gfx keeps the role of the <text> it replaces (Vega's text
 * marks have role="graphics-symbol") and adds role="img" only when the <text>
 * has neither an aria-label nor a role.
 * So a labelled mark, or a group holding one, never qualifies; a labelled
 * group of plain paths does, whatever its role. `key` is the element's
 * aria-label (normalised) or, for a <text> without one, its content (for a
 * stamped group without an aria-label, its stamped string).
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
      if (top?.outline && top.outline.valid) {
        const { key, faces } = top.outline;
        out.push(faces ? { key, weight: top.weight, style: top.style, faces } : { key, weight: top.weight, style: top.style });
      }
      continue;
    }
    const own = declared(whole, stack[stack.length - 1]);
    const label = /\saria-label\s*=\s*(?:"([^"]*)"|'([^']*)')/.exec(whole);
    const labelKey = label ? normaliseText(label[1] ?? label[2]) : undefined;
    // Any element inside an unstamped outline candidate other than an
    // unlabelled <path> or <g> means it isn't an outline group.
    if (!(name === "path" || name === "g") || label || /\srole\s*=/.test(whole)) {
      for (const e of stack) if (e.outline && !e.outline.faces) e.outline.valid = false;
    }
    if (name === "text" && !selfClosing && !open) open = { start: m.index + whole.length, label: labelKey || undefined, ...own, depth: stack.length };
    const stamp = name === "g" ? attrOf(whole, "data-surimono-text") : undefined;
    const key = labelKey || (stamp === undefined ? undefined : normaliseText(stamp));
    const faces = stamp === undefined ? undefined : splitFaces(decodeXml(attrOf(whole, "data-surimono-face") ?? ""));
    const outline = outlines && name === "g" && key && !open && !selfClosing ? { key, valid: true, faces } : undefined;
    if (!selfClosing) stack.push({ ...own, name, outline });
  }
  return out.sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : a.weight - b.weight || (a.style < b.style ? -1 : a.style > b.style ? 1 : 0)));
}

/** data-surimono-face's list: names joined with ", " (a variable face's "[axes]" may hold commas). */
function splitFaces(value) {
  return value.split(/,\s*(?![^[]*\])/).map((f) => f.trim()).filter(Boolean);
}

const WEIGHT_NAMES = { 100: "Thin", 200: "ExtraLight", 300: "Light", 400: "Regular", 500: "Medium", 600: "SemiBold", 700: "Bold", 800: "ExtraBold", 900: "Black" };
const slanted = (style) => /^(italic|oblique)\b/.test(style);

/** A face's name as its name table spells it: family and style, e.g. "Noto Sans JP Bold", "Noto Sans Italic". */
export function faceName({ family, weight, style }) {
  const w = WEIGHT_NAMES[weight] ?? String(weight);
  return `${family} ${slanted(style) ? (weight === 400 ? "Italic" : `${w} Italic`) : w}`;
}

/** The fixture fonts (fixtures/fonts/fonts.toml) as [{ family, weight, style }]. */
export function fixtureFaces(text = fs.readFileSync(new URL("../../fonts/fonts.toml", import.meta.url), "utf8")) {
  return text
    .split(/^\[\[font\]\]\s*$/m)
    .slice(1)
    .map((block) => ({
      family: /^family\s*=\s*"([^"]+)"/m.exec(block)?.[1],
      weight: Number(/^weight\s*=\s*(\d+)/m.exec(block)?.[1]),
      style: /^style\s*=\s*"([^"]+)"/m.exec(block)?.[1] ?? "normal",
    }))
    .filter((f) => f.family && f.weight);
}

/**
 * The weight CSS font matching picks from `available` for `desired`
 * (CSS Fonts 4, section 5.2, font-weight): 400 to 500 looks up to 500, then
 * down, then above 500; below 400 looks down, then up; above 500 looks up,
 * then down.
 */
export function matchWeight(desired, available) {
  const ws = [...new Set(available)].sort((a, b) => a - b);
  if (ws.includes(desired)) return desired;
  const below = ws.filter((w) => w < desired).reverse();
  const above = ws.filter((w) => w > desired);
  if (desired >= 400 && desired <= 500) return above.find((w) => w <= 500) ?? below[0] ?? above[0];
  if (desired < 400) return below[0] ?? above[0];
  return above[0] ?? below[0];
}

/**
 * The face CSS font matching draws `family` with for Chrome's computed weight
 * and style, from the faces in `fonts`: an italic face if the family has one
 * for a slanted style, else an upright face slanted by synthesis (gfx names
 * that "(synthetic oblique)"). Undefined for a family `fonts` lacks.
 */
export function expectedFace(family, weight, style, fonts) {
  const own = fonts.filter((f) => f.family === family);
  if (!own.length) return undefined;
  const italic = own.filter((f) => slanted(f.style));
  const pool = slanted(style) && italic.length ? italic : own.filter((f) => !slanted(f.style));
  if (!pool.length) return undefined;
  const w = matchWeight(weight, pool.map((f) => f.weight));
  const face = pool.find((f) => f.weight === w);
  return faceName(face) + (slanted(style) && !slanted(face.style) ? " (synthetic oblique)" : "");
}

/**
 * The faces of `drawn` (data-surimono-face names) that aren't what CSS font
 * matching picks for Chrome's weight and style: each face's family is the
 * longest fixture family its name starts with (a per-glyph fallback, such as
 * Noto Sans JP for Japanese in a Noto Sans label, is a family of its own);
 * a name that isn't a fixture face is wrong too.
 */
function wrongFaces(drawn, weight, style, fonts) {
  const families = [...new Set(fonts.map((f) => f.family))].sort((a, b) => b.length - a.length);
  return drawn.filter((name) => {
    const family = families.find((f) => name.startsWith(`${f} `));
    return !family || name !== expectedFace(family, weight, style, fonts);
  });
}

/**
 * Compares the declared weight and style of every reference <text> with the
 * candidate's texts and outline groups, one to one per key: each reference
 * entry consumes one candidate element with its key and the same weight and
 * style. A reference entry left over is a mismatch (a wrong weight or style)
 * or, when the candidate has fewer elements with that key, missing. A
 * candidate element left over (a second copy of a label, or a key Chrome has
 * no text for) is extra. Passes when nothing is left over on either side.
 *
 * A matched outline group with gfx's face stamp is also checked against the
 * faces actually drawn: each must be the face CSS font matching picks for
 * Chrome's weight and style (expectedFace, from `fonts`, the fixture fonts by
 * default). A wrong face is a mismatch too, with `surimono` "drawn with …"
 * and `expected` the face it should have been.
 */
export function compareStyles(referenceStyles, candidateSvg, { fonts } = {}) {
  const pool = new Map();
  for (const s of svgTextStyles(candidateSvg, { outlines: true })) {
    if (!pool.has(s.key)) pool.set(s.key, []);
    pool.get(s.key).push({ style: `${s.weight} ${s.style}`, faces: s.faces });
  }
  const byKey = new Map();
  for (const r of referenceStyles) {
    if (!byKey.has(r.key)) byKey.set(r.key, []);
    byKey.get(r.key).push(r);
  }
  const mismatches = [];
  const missing = [];
  const extra = [];
  let compared = 0;
  let facesChecked = 0;
  let faceList;
  const checkFaces = (key, ref, cand) => {
    if (!cand.faces?.length) return;
    faceList ??= fonts ?? fixtureFaces();
    facesChecked++;
    const wrong = wrongFaces(cand.faces, ref.weight, ref.style, faceList);
    if (wrong.length) {
      const family = [...new Set(faceList.map((f) => f.family))].sort((a, b) => b.length - a.length).find((f) => wrong[0].startsWith(`${f} `));
      mismatches.push({ key, chrome: `${ref.weight} ${ref.style}`, surimono: `drawn with ${cand.faces.join(", ")}`, expected: family ? expectedFace(family, ref.weight, ref.style, faceList) : "a fixture face" });
    }
  };
  for (const [key, wanted] of byKey) {
    const have = [...(pool.get(key) ?? [])];
    const left = [];
    for (const r of wanted) {
      const i = have.findIndex((h) => h.style === `${r.weight} ${r.style}`);
      if (i >= 0) {
        checkFaces(key, r, have.splice(i, 1)[0]);
        compared++;
      } else left.push(r);
    }
    // Leftovers pair with the candidate's remaining elements of that key as
    // wrong weights; any beyond those are missing.
    left.forEach((r, i) => {
      if (i < have.length) {
        compared++;
        mismatches.push({ key, chrome: `${r.weight} ${r.style}`, surimono: have[i].style });
      } else missing.push({ key, chrome: `${r.weight} ${r.style}` });
    });
    // Candidate elements of that key beyond Chrome's count.
    for (const h of have.slice(left.length)) extra.push({ key, surimono: h.style });
  }
  for (const [key, have] of pool) if (!byKey.has(key)) for (const h of have) extra.push({ key, surimono: h.style });
  return { pass: mismatches.length === 0 && missing.length === 0 && extra.length === 0, compared, faces_checked: facesChecked, mismatches, missing, extra };
}
