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
// changed, missing or extra characters do.

const NAMED = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: "\u00a0" };

export function decodeXml(s) {
  return s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (m, e) => {
    if (e[0] === "#") return String.fromCodePoint(e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10));
    return NAMED[e] ?? m;
  });
}

/** Decoded, with every whitespace character removed. */
export const normaliseText = (s) => decodeXml(s.replace(/<[^>]*>/g, "")).replace(/\s+/g, "");

/** { text: [<text> contents], labels: [aria-label values and <title> contents] }, each sorted, empty strings dropped. */
export function svgTexts(svg) {
  const text = [...svg.matchAll(/<text\b[^>]*>([\s\S]*?)<\/text\s*>/g)].map((m) => normaliseText(m[1]));
  const labels = [
    ...[...svg.matchAll(/\saria-label\s*=\s*(?:"([^"]*)"|'([^']*)')/g)].map((m) => normaliseText(m[1] ?? m[2])),
    ...[...svg.matchAll(/<title\b[^>]*>([\s\S]*?)<\/title\s*>/g)].map((m) => normaliseText(m[1])),
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
 * Chrome's SVG). Passes when every reference string is found, as <text> or as
 * an outline's label, and the candidate has no <text> string Chrome lacks.
 * Labels the candidate has beyond that are reported but don't fail.
 */
export function compareTexts(reference, candidateSvg) {
  const cand = svgTexts(candidateSvg);
  const outlined = multisetMinus(cand.labels, reference.labels);
  const missing = multisetMinus(reference.text, [...cand.text, ...outlined]);
  const extra = multisetMinus(cand.text, reference.text);
  const otherLabels = multisetMinus(outlined, reference.text);
  return {
    pass: missing.length === 0 && extra.length === 0,
    reference_strings: reference.text.length,
    candidate_text: cand.text.length,
    candidate_outlined: outlined.length - otherLabels.length,
    missing,
    extra,
    other_labels: otherLabels,
  };
}
