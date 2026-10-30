// Regenerates the chart SVGs in this folder, real output from the libraries
// surimono runs. Needs: npm install echarts@5.6.0 vega@5.33.1 vega-lite@5.23.0
// Usage: node generate.js echarts > echarts-5.6.0-bar-line.svg
//        node generate.js vega > vega-lite-5.23.0-bar.svg
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
