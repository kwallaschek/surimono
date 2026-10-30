// Runs inside Chrome. Finds surimono island markers and renders each one with the
// real library, client-side, then replaces the marker with the SVG. This is the
// outside truth for what an island should look like; it never runs inside
// surimono.
//
// Marker: <script type="application/vnd.surimono+json" data-renderer="echarts|vega-lite|vega"
//                 data-width="W" data-height="H">{spec JSON}</script>
//
// Contract (see fixtures/README.md):
// - echarts: echarts.init in SSR mode with the SVG renderer, width W and height H,
//   animation forced off, then renderToSVGString().
// - vega-lite: vegaLite.compile(spec).spec, then as vega.
// - vega: new vega.View(vega.parse(spec), { renderer: "none" }), runAsync(), toSVG().
//   The spec is used verbatim; the SVG it produces must be exactly W x H, or the
//   fixture is inconsistent and the harness fails.
// Text is measured by Chrome's canvas with the page's @font-face fonts, which are
// all loaded before any island renders.

/* global echarts, vega, vegaLite */
(() => {
  const MARKER = 'script[type="application/vnd.surimono+json"]';

  function svgSize(svg) {
    const tpl = document.createElement("template");
    tpl.innerHTML = svg.trim();
    const root = tpl.content.firstElementChild;
    if (!root || root.localName !== "svg") throw new Error("renderer did not return an <svg> root");
    return { width: Number(root.getAttribute("width")), height: Number(root.getAttribute("height")) };
  }

  async function renderOne(renderer, spec, width, height) {
    switch (renderer) {
      case "echarts": {
        const chart = echarts.init(null, null, { renderer: "svg", ssr: true, width, height });
        try {
          chart.setOption({ ...spec, animation: false });
          return chart.renderToSVGString();
        } finally {
          chart.dispose();
        }
      }
      case "vega-lite":
        return renderVega(vegaLite.compile(spec).spec);
      case "vega":
        return renderVega(spec);
      default:
        throw new Error(`unknown renderer "${renderer}"`);
    }
  }

  async function renderVega(spec) {
    const view = new vega.View(vega.parse(spec), { renderer: "none", logLevel: vega.Warn });
    try {
      await view.runAsync();
      return await view.toSVG();
    } finally {
      view.finalize();
    }
  }

  // Every font an island names must be one of the fixture fonts; a spec that
  // falls back to a system font (e.g. ECharts' default sans-serif) is a broken
  // fixture, because surimono and fulgur would pick a different face.
  const ALLOWED_FAMILIES = new Set(["Noto Sans", "Noto Sans JP"]);
  function foreignFonts(svg) {
    const found = new Set();
    for (const m of svg.matchAll(/font-family\s*(?:=\s*"([^"]*)"|:\s*([^;"]*))/g)) {
      for (const fam of (m[1] ?? m[2]).replace(/&quot;/g, '"').split(",")) {
        const name = fam.trim().replace(/^['"]|['"]$/g, "");
        if (name && !ALLOWED_FAMILIES.has(name)) found.add(name);
      }
    }
    return [...found];
  }

  async function loadAllFonts() {
    const faces = [...document.fonts];
    await Promise.all(faces.map((f) => f.load()));
    await document.fonts.ready;
    const failed = faces.filter((f) => f.status !== "loaded");
    if (failed.length) throw new Error(`fonts failed to load: ${failed.map((f) => `${f.family} ${f.weight}`).join(", ")}`);
    return faces.map((f) => `${f.family.replace(/"/g, "")} ${f.weight} ${f.style}`);
  }

  window.__surimonoRenderIslands = async () => {
    const fonts = await loadAllFonts();
    const markers = [...document.querySelectorAll(MARKER)];
    const islands = [];
    for (const [index, el] of markers.entries()) {
      const renderer = el.dataset.renderer;
      const width = Number(el.dataset.width);
      const height = Number(el.dataset.height);
      if (!(width > 0 && height > 0)) throw new Error(`island ${index}: data-width and data-height must be positive numbers`);
      const spec = JSON.parse(el.textContent);
      const svg = await renderOne(renderer, spec, width, height);
      const foreign = foreignFonts(svg);
      if (foreign.length) throw new Error(`island ${index} (${renderer}): SVG uses fonts outside fixtures/fonts: ${foreign.join(", ")}`);
      const size = svgSize(svg);
      if (size.width !== width || size.height !== height) {
        throw new Error(`island ${index} (${renderer}): SVG is ${size.width}x${size.height}, marker says ${width}x${height}`);
      }
      const tpl = document.createElement("template");
      tpl.innerHTML = svg.trim();
      el.replaceWith(tpl.content);
      islands.push({ index, renderer, width, height, svg });
    }
    // Text drawn after the island SVGs are in the DOM must use loaded fonts too.
    await document.fonts.ready;
    return { fonts, islands };
  };
})();
