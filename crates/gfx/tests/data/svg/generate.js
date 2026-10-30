// Regenerates the chart SVGs in this folder, real output from the libraries
// surimono runs. Needs: npm install echarts@5.6.0 vega@5.33.1 vega-lite@5.23.0
// (and echarts@6.1.0, the version crates/islands bundles, for echarts-rich)
// Usage: node generate.js echarts > echarts-5.6.0-bar-line.svg
//        node generate.js vega > vega-lite-5.23.0-bar.svg
//        node generate.js echarts-rich > echarts-6.1.0-rich.svg
const which = process.argv[2];
if (which === 'echarts') {
  const echarts = require('echarts');
  const chart = echarts.init(null, null, { renderer: 'svg', ssr: true, width: 400, height: 300 });
  chart.setOption({
    animation: false, title: { text: '売上 Sales' }, tooltip: {}, legend: { data: ['A'] },
    xAxis: { data: ['Mon', 'Tue', '水'] }, yAxis: {},
    series: [
      { name: 'A', type: 'bar', data: [5, 20, 36], itemStyle: { color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [{ offset: 0, color: '#83bff6' }, { offset: 1, color: '#188df0' }]) } },
      { type: 'line', data: [3, 8, 12], areaStyle: {} },
    ],
  });
  console.log(chart.renderToSVGString());
  process.exit(0); // the tooltip keeps a timer alive
} else if (which === 'echarts-rich') {
  // Every kind of id ECharts emits: gradients, clip paths, decal patterns and
  // shadow filters, with Latin and Japanese labels in several baselines.
  const echarts = require('echarts');
  const chart = echarts.init(null, null, { renderer: 'svg', ssr: true, width: 480, height: 320 });
  chart.setOption({
    animation: false, aria: { enabled: true, decal: { show: true } },
    title: { text: '月別売上 Monthly', subtext: '単位: 百万円', left: 'center' },
    legend: { data: ['売上', 'Cost'], top: 40 },
    grid: { top: 80 },
    xAxis: { data: ['1月', '2月', '3月', 'Apr'], axisLabel: { rotate: 30 } },
    yAxis: { name: '金額', nameLocation: 'middle', nameGap: 30 },
    series: [
      { name: '売上', type: 'bar', data: [120, 200, 150, 80], label: { show: true, position: 'top' },
        itemStyle: { shadowBlur: 6, shadowColor: 'rgba(0,0,0,0.3)',
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [{ offset: 0, color: '#83bff6' }, { offset: 1, color: '#188df0' }]) } },
      { name: 'Cost', type: 'line', data: [60, 90, 110, 70], areaStyle: { opacity: 0.3 }, smooth: true },
    ],
  });
  console.log(chart.renderToSVGString());
  process.exit(0);
} else if (which === 'vega') {
  const vega = require('vega');
  const vl = require('vega-lite');
  const spec = vl.compile({
    $schema: 'https://vega.github.io/schema/vega-lite/v5.json', title: '月別 Revenue',
    data: { values: [{ a: 'A', b: 28 }, { a: 'B', b: 55 }, { a: '日本', b: 43 }] },
    mark: { type: 'bar', tooltip: true },
    encoding: { x: { field: 'a', type: 'nominal' }, y: { field: 'b', type: 'quantitative' }, color: { field: 'a' } },
  }).spec;
  new vega.View(vega.parse(spec), { renderer: 'none' }).toSVG().then((svg) => { console.log(svg); process.exit(0); });
} else {
  console.error('usage: node generate.js echarts|vega');
  process.exit(2);
}
