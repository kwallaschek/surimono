// The built-in `vega` and `vega-lite` island renderers, bundled by
// ../build.sh into one script with two entries:
//
//   vegaEntry(specJson, ctx)      the spec is a Vega spec
//   vegaLiteEntry(specJson, ctx)  the spec is a Vega-Lite spec, compiled to
//                                 Vega with vega-lite's compile()
//
// The spec is used as it is, including its own width, height, padding and
// autosize, so the SVG has the size the spec asks for (as in a browser).
// The one exception is Vega-Lite's "container" width or height, which
// means the island's box.
// The chart is rendered by a headless vega.View (renderer 'none') and
// serialized with view.toSVG(); text is measured by the host through
// ctx.measureText (see vega-canvas.js). Data must be inline: anything the
// spec would load (a data `url`, an image) is refused, and the render fails
// with that error instead of drawing a chart without its data.

import './timers.js';
import './structured-clone.js';
import { View, Warn, loader as vegaLoader, logger as vegaLogger, parse } from 'vega';
import { compile } from 'vega-lite';
import { setMeasureText } from './vega-canvas.js';

function parseSpec(specJson, what) {
  const spec = JSON.parse(specJson);
  if (spec === null || typeof spec !== 'object' || Array.isArray(spec)) {
    throw new TypeError(`${what}: the island spec must be a ${what} spec object`);
  }
  return spec;
}

// A loader that loads nothing. Vega calls sanitize() before any request.
// Vega only warns about a failed data load and draws the chart without that
// data, so each refusal is also recorded, and the render fails with it.
// Images and links are refused synchronously: Vega's SVG renderer waits for
// them by polling on a timer, and island timers never fire, so a refusal
// that arrives later would leave the render waiting forever.
function noLoader(refusals) {
  const loader = vegaLoader();
  loader.sanitize = (uri, options) => {
    const context = options && options.context;
    const target = JSON.stringify(String(uri));
    let error;
    if (context === 'image') {
      error = new Error(
        `surimono: islands have no network or file access, so Vega image marks ` +
        `are not supported; this spec loads the image ${target}`,
      );
    } else if (context === 'href') {
      error = new Error(
        `surimono: Vega links (href) are not supported in islands; this spec links ${target}`,
      );
    } else {
      error = new Error(
        `surimono: islands have no network or file access, so Vega data must be ` +
        `inline ("values"); this spec loads ${target}`,
      );
    }
    refusals.push(error.message);
    if (context === 'image' || context === 'href') throw error;
    return Promise.reject(error);
  };
  return loader;
}

async function render(vegaSpec, ctx) {
  // Vega reports most failures (a failed load, a bad expression) to its
  // logger and carries on; collect them and fail the render instead.
  const errors = [];
  const logger = vegaLogger(Warn, undefined, (method, level, args) => {
    if (level === 'ERROR') errors.push(args.map(String).join(' '));
  });
  const refusals = [];
  const failOnErrors = () => {
    if (refusals.length) throw new Error(refusals[0]);
    if (errors.length) throw new Error(errors.join('; '));
  };

  setMeasureText((text, font) => ctx.measureText(text, font));
  let view = null;
  try {
    view = new View(parse(vegaSpec), { renderer: 'none', logger, loader: noLoader(refusals) });
    await view.runAsync();
    failOnErrors();
    const svg = await view.toSVG();
    failOnErrors();
    return svg;
  } finally {
    if (view) view.finalize();
    setMeasureText(null);
  }
}

function vegaEntry(specJson, ctx) {
  return render(parseSpec(specJson, 'vega'), ctx);
}

// "container" sizes a Vega-Lite chart to its element in a browser; here the
// element is the island's box. Fit the whole view, padding included, to it,
// as Vega-Lite does for a container of that size.
function containerToIsland(spec, ctx) {
  const w = spec.width === 'container';
  const h = spec.height === 'container';
  if (!w && !h) return;
  if (w) spec.width = ctx.width;
  if (h) spec.height = ctx.height;
  if (spec.autosize === undefined) {
    spec.autosize = { type: w && h ? 'fit' : w ? 'fit-x' : 'fit-y', contains: 'padding' };
  }
}

function vegaLiteEntry(specJson, ctx) {
  const spec = parseSpec(specJson, 'vega-lite');
  containerToIsland(spec, ctx);
  return render(compile(spec).spec, ctx);
}

globalThis.vegaEntry = vegaEntry;
globalThis.vegaLiteEntry = vegaLiteEntry;
