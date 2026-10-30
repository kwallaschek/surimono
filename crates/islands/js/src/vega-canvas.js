// Stands in for the `vega-canvas` package in the Vega bundle (build.sh
// aliases it here).
//
// Vega asks vega-canvas for one 1x1 canvas when it loads and measures all
// text with that canvas's 2D context. Here that context forwards to the
// host's `ctx.measureText`, so Vega lays out labels with the same font
// metrics as ECharts and the PDF engine, through Vega's own code path
// (caching, truncation with `limit`), exactly as in a browser.
//
// Islands have no raster canvas: the Vega features that draw pixels (the
// label transform, word clouds, heatmap images) get a clear error. Image
// marks get no Image constructor, as in Node.

let measure = null;

/** Sets the function text is measured with during a render, or null. */
export function setMeasureText(fn) {
  measure = fn;
}

const measuringContext = {
  font: '10px sans-serif',
  measureText(text) {
    if (measure === null) {
      throw new Error('surimono: Vega measured text outside a render');
    }
    return measure(String(text), this.font);
  },
};

let textCanvasGiven = false;

export function canvas(width, height) {
  if (!textCanvasGiven && width === 1 && height === 1) {
    textCanvasGiven = true;
    return { width: 1, height: 1, getContext: () => measuringContext };
  }
  throw new Error(
    'surimono: this Vega spec needs a raster canvas (label transform, word cloud ' +
    'or heatmap image), which islands do not support',
  );
}

export const domCanvas = canvas;

export function image() {
  return null;
}

export const domImage = image;
