// The built-in `echarts` island renderer, bundled by ../build.sh.
//
// The island spec is an ECharts option object as JSON. The chart is drawn
// with ECharts' own server-side SVG renderer (no DOM, no canvas), with every
// animation off so the SVG shows the chart's final state, and with text
// measured by the host through `ctx.measureText`, so labels are laid out with
// the fonts the PDF engine will print with.
//
// Everything in the full `echarts` package is registered except the canvas
// renderer and the universal-transition animation feature, which do nothing
// in a static SVG.

import './timers.js';
import * as echarts from 'echarts/core';
import { SVGRenderer } from 'echarts/renderers';
import * as charts from 'echarts/charts';
import * as components from 'echarts/components';
import {
  AxisBreak,
  LabelLayout,
  LegacyGridContainLabel,
  ScatterJitter,
} from 'echarts/features';

echarts.use([SVGRenderer]);
echarts.use(Object.values(charts));
echarts.use(Object.values(components));
echarts.use([LabelLayout, AxisBreak, LegacyGridContainLabel, ScatterJitter]);

function isObject(v) {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

// Turns animation off for the option and for each series in it. A series
// may switch animation back on for itself, and in a static render that
// would freeze it at its first frame.
function disableAnimation(option) {
  if (!isObject(option)) return;
  option.animation = false;
  const series = option.series;
  if (Array.isArray(series)) {
    for (const s of series) {
      if (isObject(s)) s.animation = false;
    }
  } else if (isObject(series)) {
    series.animation = false;
  }
}

function positiveSize(v, what) {
  if (typeof v !== 'number' || !(v > 0)) {
    throw new TypeError(`echarts: the island needs a positive ${what}, got ${v}`);
  }
  return v;
}

function entry(specJson, ctx) {
  const width = positiveSize(ctx.width, 'width');
  const height = positiveSize(ctx.height, 'height');
  const option = JSON.parse(specJson);
  if (!isObject(option)) {
    throw new TypeError('echarts: the island spec must be an ECharts option object');
  }
  // A timeline option keeps its parts under baseOption and options, and a
  // responsive one under media[].option.
  disableAnimation(option);
  disableAnimation(option.baseOption);
  if (Array.isArray(option.options)) option.options.forEach(disableAnimation);
  if (Array.isArray(option.media)) {
    for (const m of option.media) {
      if (isObject(m)) disableAnimation(m.option);
    }
  }

  echarts.setPlatformAPI({
    measureText: (text, font) => ctx.measureText(text, font),
  });
  const chart = echarts.init(null, null, {
    renderer: 'svg',
    ssr: true,
    width,
    height,
    locale: 'EN',
  });
  try {
    chart.setOption(option);
    return chart.renderToSVGString();
  } finally {
    chart.dispose();
  }
}

globalThis.entry = entry;
