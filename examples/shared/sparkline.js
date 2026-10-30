// A small renderer of your own: a sparkline with a measured label.
//
// surimono runs this as a classic script in its sandbox. It defines one global
// function, `render(specJson, ctx)`, that turns the island's JSON into an SVG
// string. `ctx` gives the island's size and `measureText`, which measures with
// the fonts the renderer was built with. There is no DOM, console or timer.

globalThis.render = function render(specJson, ctx) {
  const spec = JSON.parse(specJson);
  const values = spec.values;
  const label = typeof spec.label === "string" ? spec.label : "";
  if (!Array.isArray(values) || values.length === 0) {
    throw new TypeError("sparkline: spec.values must be a non-empty array of numbers");
  }

  const font = "12px 'Noto Sans'";
  const text = label ? ctx.measureText(label, font) : null;
  const plotWidth = ctx.width - (text ? text.width + 6 : 0);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const x = (i) => (i * plotWidth) / Math.max(values.length - 1, 1);
  const y = (v) => ctx.height - 1 - ((v - min) / (max - min || 1)) * (ctx.height - 2);
  const points = values.map((v, i) => `${x(i).toFixed(2)},${y(v).toFixed(2)}`).join(" ");

  let svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${ctx.width}" height="${ctx.height}" ` +
    `viewBox="0 0 ${ctx.width} ${ctx.height}">` +
    `<polyline fill="none" stroke="#2f6fb0" stroke-width="1.5" points="${points}"/>`;
  if (text) {
    const baseline = (ctx.height + text.actualBoundingBoxAscent) / 2;
    svg +=
      `<text x="${ctx.width}" y="${baseline.toFixed(2)}" text-anchor="end" ` +
      `font-family="Noto Sans" font-size="12">${escapeXml(label)}</text>`;
  }
  return svg + "</svg>";
};

function escapeXml(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
