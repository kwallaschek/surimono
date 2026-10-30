// Island fixture definitions. Each entry becomes fixtures/islands/<name>/.
// Fonts: every spec names its fonts explicitly; nothing may fall back to a
// system font. Vega and Vega-Lite specs fit the marker box exactly
// (autosize fit, contains padding), which the Chrome harness asserts.

import { fmtInt, isoDays, rng, round, walk } from "./data.mjs";

const LATIN = "'Noto Sans'";
const JA = "'Noto Sans JP'";
// Latin first, Japanese by per-glyph fallback (tests the fallback chain).
const LATIN_THEN_JA = "'Noto Sans', 'Noto Sans JP'";
const PAGE_LATIN = '"Noto Sans", "Noto Sans JP"';
const PAGE_JA = '"Noto Sans JP", "Noto Sans"';
const PALETTE = ["#2f6fb0", "#e07b39", "#3a9b6a", "#c2455a", "#7a5cb8", "#8c8c8c", "#d4a72c", "#4bb3c4"];

const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function echartsBarSimpleLatin() {
  const next = rng(101);
  const revenue = months.map((_, i) => Math.round(42000 + i * 1800 + next() * 9000));
  return {
    name: "echarts-bar-simple-latin",
    lang: "en",
    title: "Monthly invoiced revenue, FY2026",
    org: "Northwind Trading GmbH",
    headerMeta: ["Revenue report", "Period: Jan to Dec 2026", "Currency: EUR"],
    pageFont: PAGE_LATIN,
    fonts: ["Noto Sans 400", "Noto Sans 700"],
    intro: "Invoiced revenue per month, net of credit notes. Values are in euros and rounded to the nearest euro.",
    tests: "ECharts SVG: one bar series, category axis, value labels above bars, title; Latin text in Noto Sans.",
    islands: [
      {
        renderer: "echarts",
        width: 640,
        height: 320,
        caption: "Figure 1. Invoiced revenue per month (EUR).",
        spec: {
          textStyle: { fontFamily: LATIN },
          title: { text: "Invoiced revenue (EUR)", left: 0, top: 0, textStyle: { fontFamily: LATIN, fontSize: 14, fontWeight: "bold" } },
          grid: { left: 64, right: 16, top: 48, bottom: 32 },
          xAxis: { type: "category", data: months, axisTick: { alignWithLabel: true } },
          yAxis: { type: "value", axisLabel: { formatter: "{value}" } },
          series: [
            {
              type: "bar",
              name: "Revenue",
              data: revenue,
              barWidth: "60%",
              itemStyle: { color: "#2f6fb0" },
              label: { show: true, position: "top", fontSize: 9, fontFamily: LATIN },
            },
          ],
        },
      },
    ],
    table: {
      head: ["Quarter", "Revenue (EUR)"],
      numeric: [false, true],
      rows: [0, 1, 2, 3].map((q) => [`Q${q + 1}`, fmtInt(revenue.slice(q * 3, q * 3 + 3).reduce((a, b) => a + b, 0))]),
    },
  };
}

function vegaliteBarSimpleLatin() {
  const next = rng(202);
  const customers = ["Alpine Foods AG", "Baltic Shipping", "Cobalt Systems", "Delta Pharma", "Elm Street Books", "Fjord Energy", "Granite Labs", "Harbor Retail"];
  // Pre-sorted, so the chart can use the data order (sort: null) in both layers.
  const values = customers.map((c) => ({ customer: c, total: Math.round(8000 + next() * 52000) })).sort((a, b) => b.total - a.total);
  return {
    name: "vegalite-bar-simple-latin",
    lang: "en",
    title: "Open invoices by customer",
    org: "Northwind Trading GmbH",
    headerMeta: ["Accounts receivable", "As of 31 Oct 2026", "Currency: EUR"],
    pageFont: PAGE_LATIN,
    fonts: ["Noto Sans 400", "Noto Sans 700"],
    intro: "Total of unpaid invoices per customer, sorted by amount. Includes invoices not yet due.",
    tests: "Vega-Lite SVG: horizontal layered bar and text chart in data order, nominal y axis with long labels, quantitative x axis with grid, text labels; autosize fit.",
    islands: [
      {
        renderer: "vega-lite",
        width: 640,
        height: 304,
        caption: "Figure 1. Open invoice total per customer (EUR).",
        spec: {
          $schema: "https://vega.github.io/schema/vega-lite/v6.json",
          width: 640,
          height: 304,
          autosize: { type: "fit", contains: "padding" },
          padding: 4,
          config: { font: "Noto Sans", axis: { labelFontSize: 10, titleFontSize: 11 }, view: { stroke: null } },
          data: { values },
          encoding: {
            y: { field: "customer", type: "nominal", sort: null, title: null },
            x: { field: "total", type: "quantitative", title: "Open amount (EUR)", axis: { format: ",d" } },
          },
          layer: [
            { mark: { type: "bar", color: "#2f6fb0" } },
            {
              mark: { type: "text", align: "left", dx: 3, fontSize: 9 },
              encoding: { text: { field: "total", type: "quantitative", format: ",d" } },
            },
          ],
        },
      },
    ],
    table: {
      head: ["Customer", "Open amount (EUR)"],
      numeric: [false, true],
      rows: values.map((v) => [v.customer, fmtInt(v.total)]),
    },
  };
}

function echartsLineDenseLatin() {
  const next = rng(303);
  const days = isoDays("2025-01-01", 365);
  const channels = [
    ["Web shop", 820, 0.9, 140],
    ["Marketplace", 610, 0.5, 120],
    ["Retail", 540, -0.2, 90],
    ["Wholesale", 380, 0.3, 160],
    ["Phone orders", 210, -0.1, 60],
  ];
  const series = channels.map(([name, base, drift, noise]) => {
    const values = walk(next, days.length, base, drift, noise);
    // Weekly rhythm: weekends sell less except on the web shop.
    return { name, data: values.map((v, i) => Math.round(v * ((i + 3) % 7 >= 5 && name !== "Web shop" ? 0.7 : 1))) };
  });
  const t0 = Date.UTC(2025, 0, 1);
  return {
    name: "echarts-line-dense-latin",
    lang: "en",
    title: "Daily orders by sales channel, 2025",
    org: "Northwind Trading GmbH",
    headerMeta: ["Sales report", "Period: 1 Jan to 31 Dec 2025", "Unit: orders per day"],
    pageFont: PAGE_LATIN,
    fonts: ["Noto Sans 400", "Noto Sans 700"],
    intro: "Orders received per day and channel. Weekend dips are expected for retail, wholesale and phone orders.",
    tests: "ECharts SVG, dense: 5 line series x 365 points without symbols, legend, a UTC time axis (useUTC, numeric timestamps) whose ticks and labels ECharts chooses itself, grid lines; Latin text.",
    islands: [
      {
        renderer: "echarts",
        width: 680,
        height: 384,
        caption: "Figure 1. Orders per day and channel (5 series, 365 days each).",
        spec: {
          useUTC: true,
          color: PALETTE,
          textStyle: { fontFamily: LATIN },
          legend: { top: 0, left: "center", itemWidth: 18, itemHeight: 8, textStyle: { fontFamily: LATIN, fontSize: 11 } },
          grid: { left: 48, right: 24, top: 36, bottom: 32 },
          xAxis: { type: "time", min: t0, max: t0 + 364 * 86400000, axisLabel: { fontSize: 10 } },
          yAxis: { type: "value", name: "Orders", nameTextStyle: { fontSize: 10 }, splitLine: { lineStyle: { color: "#e5e8eb" } } },
          series: series.map((s) => ({ type: "line", name: s.name, data: s.data.map((v, i) => [t0 + i * 86400000, v]), showSymbol: false, lineStyle: { width: 1.2 } })),
        },
      },
    ],
    table: {
      head: ["Channel", "Orders", "Daily mean", "Peak day"],
      numeric: [false, true, true, true],
      rows: series.map((s) => {
        const total = s.data.reduce((a, b) => a + b, 0);
        return [s.name, fmtInt(total), fmtInt(total / s.data.length), fmtInt(Math.max(...s.data))];
      }),
    },
  };
}

function echartsBarSimpleJa() {
  const next = rng(404);
  const depts = ["営業部", "開発部", "マーケティング部", "カスタマーサクセス部", "経理部", "人事部"];
  const budget = depts.map(() => Math.round(800 + next() * 2400));
  const actual = budget.map((b) => Math.round(b * (0.82 + next() * 0.3)));
  return {
    name: "echarts-bar-simple-ja",
    lang: "ja",
    title: "部門別 予算と実績（2026年度上期）",
    org: "株式会社みなと商事",
    headerMeta: ["経営管理レポート", "対象期間：2026年4月〜9月", "単位：万円"],
    pageFont: PAGE_JA,
    fonts: ["Noto Sans JP 400", "Noto Sans JP 700"],
    intro: "各部門の上期予算と実績を比較しています。実績は9月末時点の確定値です。",
    tests: "ECharts SVG: two bar series with legend, Japanese category labels where the long ones wrap by measured width (axisLabel width 84 px, overflow break), Japanese title and axis name; all text in Noto Sans JP.",
    islands: [
      {
        renderer: "echarts",
        width: 640,
        height: 344,
        caption: "図1　部門別の予算と実績（万円）",
        spec: {
          color: ["#9db7d5", "#2f6fb0"],
          textStyle: { fontFamily: JA },
          title: { text: "予算と実績", left: 0, top: 0, textStyle: { fontFamily: JA, fontSize: 14, fontWeight: "bold" } },
          legend: { top: 2, right: 0, data: ["予算", "実績"], textStyle: { fontFamily: JA, fontSize: 11 } },
          grid: { left: 56, right: 8, top: 48, bottom: 52 },
          xAxis: { type: "category", data: depts, axisLabel: { fontSize: 11, interval: 0, width: 84, overflow: "break", lineHeight: 15 } },
          yAxis: { type: "value", name: "万円", nameTextStyle: { fontSize: 11 } },
          series: [
            { type: "bar", name: "予算", data: budget, barGap: "10%" },
            { type: "bar", name: "実績", data: actual, label: { show: true, position: "top", fontSize: 9, fontFamily: JA } },
          ],
        },
      },
    ],
    table: {
      head: ["部門", "予算（万円）", "実績（万円）", "達成率"],
      numeric: [false, true, true, true],
      rows: depts.map((d, i) => [d, fmtInt(budget[i]), fmtInt(actual[i]), `${round((actual[i] / budget[i]) * 100, 1).toFixed(1)}%`]),
    },
  };
}

function echartsPieJa() {
  const next = rng(505);
  const items = ["人件費", "外注費", "地代家賃", "広告宣伝費", "通信費", "旅費交通費", "消耗品費", "その他"];
  const weights = [42, 18, 12, 9, 5, 4, 3, 7];
  const values = weights.map((w) => Math.round(w * (95 + next() * 10) * 10));
  const total = values.reduce((a, b) => a + b, 0);
  return {
    name: "echarts-pie-ja",
    lang: "ja",
    title: "販売費及び一般管理費の内訳（2026年9月）",
    org: "株式会社みなと商事",
    headerMeta: ["月次決算レポート", "対象月：2026年9月", "単位：千円"],
    pageFont: PAGE_JA,
    fonts: ["Noto Sans JP 400", "Noto Sans JP 700"],
    intro: "9月の販売費及び一般管理費を科目別に示します。人件費が全体の約4割を占めています。",
    tests: "ECharts SVG: pie with outside labels and leader lines, '{b}\\n{d}%' label template (two-line labels with percentages), vertical legend; Japanese text in Noto Sans JP.",
    islands: [
      {
        renderer: "echarts",
        width: 640,
        height: 360,
        caption: "図1　科目別の構成比",
        spec: {
          color: PALETTE,
          textStyle: { fontFamily: JA },
          legend: { orient: "vertical", right: 0, top: "middle", itemWidth: 12, itemHeight: 12, textStyle: { fontFamily: JA, fontSize: 11 } },
          series: [
            {
              type: "pie",
              name: "費用",
              center: ["40%", "52%"],
              radius: ["0%", "62%"],
              data: items.map((name, i) => ({ name, value: values[i] })),
              label: { formatter: "{b}\n{d}%", fontSize: 11, fontFamily: JA, lineHeight: 15 },
              labelLine: { length: 12, length2: 10 },
              itemStyle: { borderColor: "#ffffff", borderWidth: 1 },
            },
          ],
        },
      },
    ],
    table: {
      head: ["科目", "金額（千円）", "構成比"],
      numeric: [false, true, true],
      rows: [...items.map((name, i) => [name, fmtInt(values[i]), `${round((values[i] / total) * 100, 1).toFixed(1)}%`]), ["合計", fmtInt(total), "100.0%"]],
    },
  };
}

function echartsMixedDenseJa() {
  const next = rng(606);
  const labels = [];
  for (let y = 2024; y <= 2026; y++) for (let m = 1; m <= 12; m++) labels.push(`${y}年${m}月`);
  const products = ["クラウド会計", "給与計算", "経費精算"];
  const bars = products.map((_, p) => walk(next, labels.length, 1200 - p * 300, 18 - p * 4, 90 - p * 20));
  const totals = labels.map((_, i) => bars.reduce((a, b) => a + b[i], 0));
  const margin = totals.map((t, i) => round(14 + (i / labels.length) * 8 + (next() - 0.5) * 3, 1));
  return {
    name: "echarts-mixed-dense-ja",
    lang: "ja",
    title: "製品別売上高と営業利益率の推移",
    org: "Minato Software 株式会社",
    headerMeta: ["Quarterly business review / 四半期報告", "期間：2024年1月〜2026年12月", "売上高：百万円、利益率：%"],
    // Japanese first on the page: fulgur 0.40.0 does not grow line boxes for
    // fallback glyphs, which would move the island. The island itself keeps
    // the Latin-first fallback chain.
    pageFont: PAGE_JA,
    fonts: ["Noto Sans 400", "Noto Sans 700", "Noto Sans JP 400", "Noto Sans JP 700"],
    intro: "Revenue by product (stacked bars, left axis) and operating margin (line, right axis) over 36 months. 製品別の月次売上高と営業利益率の推移です。",
    tests: "ECharts SVG, dense: 3 stacked bar series + 1 line on a second y axis (36 months, 144 marks), markLine average with label, rotated (45 degree) mixed Japanese/digit axis labels, legend; font list 'Noto Sans', 'Noto Sans JP' so Japanese glyphs fall back per glyph.",
    islands: [
      {
        renderer: "echarts",
        width: 680,
        height: 400,
        caption: "Figure 1 / 図1　製品別売上高（積み上げ）と営業利益率",
        spec: {
          color: ["#2f6fb0", "#6fa0d3", "#b5cde8", "#c2455a"],
          textStyle: { fontFamily: LATIN_THEN_JA },
          legend: { top: 0, left: "center", textStyle: { fontFamily: LATIN_THEN_JA, fontSize: 11 } },
          grid: { left: 56, right: 48, top: 40, bottom: 72 },
          xAxis: { type: "category", data: labels, axisLabel: { rotate: 45, interval: 2, fontSize: 10 } },
          yAxis: [
            { type: "value", name: "売上高（百万円）", nameTextStyle: { fontSize: 10, align: "left" }, splitLine: { lineStyle: { color: "#e5e8eb" } } },
            { type: "value", name: "利益率", min: 0, max: 30, axisLabel: { formatter: "{value}%" }, nameTextStyle: { fontSize: 10 }, splitLine: { show: false } },
          ],
          series: [
            ...products.map((name, p) => ({ type: "bar", name, stack: "revenue", data: bars[p], barWidth: "62%" })),
            {
              type: "line",
              name: "営業利益率",
              yAxisIndex: 1,
              data: margin,
              symbol: "circle",
              symbolSize: 4,
              lineStyle: { width: 1.5 },
              markLine: { symbol: "none", data: [{ type: "average", name: "平均" }], label: { formatter: "平均 {c}%", position: "insideEndTop", fontFamily: LATIN_THEN_JA, fontSize: 10 }, lineStyle: { type: "dashed" } },
            },
          ],
        },
      },
    ],
    table: {
      head: ["年度 / Year", "売上高（百万円）", "平均利益率"],
      numeric: [false, true, true],
      rows: [0, 1, 2].map((y) => [
        `${2024 + y}年`,
        fmtInt(totals.slice(y * 12, y * 12 + 12).reduce((a, b) => a + b, 0)),
        `${round(margin.slice(y * 12, y * 12 + 12).reduce((a, b) => a + b, 0) / 12, 1).toFixed(1)}%`,
      ]),
    },
  };
}

function vegaliteScatterDenseLatin() {
  const next = rng(707);
  const segments = ["Enterprise", "SMB"];
  const values = Array.from({ length: 2000 }, (_, i) => {
    const segment = segments[i % 4 === 0 ? 0 : 1];
    const amount = round(segment === "Enterprise" ? 4000 + next() ** 0.6 * 46000 : 200 + next() ** 1.6 * 9800, 0);
    const base = segment === "Enterprise" ? 38 : 21;
    const days = Math.max(0, Math.round(base + (next() + next() + next() - 1.5) * 18 + amount / 4000));
    return { amount, days, segment };
  });
  const stat = (seg) => {
    const v = values.filter((x) => x.segment === seg);
    return [seg, fmtInt(v.length), fmtInt(v.reduce((a, b) => a + b.amount, 0) / v.length), `${round(v.reduce((a, b) => a + b.days, 0) / v.length, 1).toFixed(1)}`];
  };
  return {
    name: "vegalite-scatter-dense-latin",
    lang: "en",
    title: "Invoice amount and days to payment",
    org: "Northwind Trading GmbH",
    headerMeta: ["Credit control", "Invoices paid Jan to Sep 2026", "n = 2,000"],
    pageFont: PAGE_LATIN,
    fonts: ["Noto Sans 400", "Noto Sans 700"],
    intro: "Each point is one paid invoice. Enterprise customers pay larger invoices and take longer to pay on average.",
    tests: "Vega-Lite SVG, dense: 2,000 circle marks with opacity, nominal colour with legend, log-scaled x axis, axis titles, grid; Latin text.",
    islands: [
      {
        renderer: "vega-lite",
        width: 640,
        height: 400,
        caption: "Figure 1. Invoice amount (log scale) against days from invoice to payment, 2,000 invoices.",
        spec: {
          $schema: "https://vega.github.io/schema/vega-lite/v6.json",
          width: 640,
          height: 400,
          autosize: { type: "fit", contains: "padding" },
          padding: 4,
          config: { font: "Noto Sans", axis: { labelFontSize: 10, titleFontSize: 11, gridColor: "#e5e8eb" }, legend: { labelFontSize: 10, titleFontSize: 11 }, view: { stroke: null } },
          data: { values },
          mark: { type: "circle", size: 14, opacity: 0.55 },
          encoding: {
            x: { field: "amount", type: "quantitative", scale: { type: "log", domain: [100, 100000] }, title: "Invoice amount (EUR, log scale)", axis: { format: ",d" } },
            y: { field: "days", type: "quantitative", title: "Days to payment", scale: { domain: [0, 90] } },
            color: { field: "segment", type: "nominal", title: "Segment", scale: { range: ["#c2455a", "#2f6fb0"] } },
          },
        },
      },
    ],
    table: { head: ["Segment", "Invoices", "Avg. EUR", "Avg. days"], numeric: [false, true, true, true], rows: segments.map(stat) },
  };
}

function vegaliteLineJa() {
  const next = rng(808);
  const stores = ["渋谷店", "新宿店", "横浜店"];
  const values = [];
  for (const [s, store] of stores.entries()) {
    const series = walk(next, 12, 3200 - s * 600, 40, 260);
    series.forEach((v, m) => values.push({ month: `2025-${String(m + 1).padStart(2, "0")}-01`, store, sales: v }));
  }
  return {
    name: "vegalite-line-ja",
    lang: "ja",
    title: "店舗別 月次売上高の推移（2025年）",
    org: "株式会社みなと商事",
    headerMeta: ["店舗運営レポート", "対象期間：2025年1月〜12月", "単位：万円"],
    pageFont: PAGE_JA,
    fonts: ["Noto Sans JP 400", "Noto Sans JP 700"],
    intro: "首都圏3店舗の月次売上高です。横浜店は夏季に売上が伸びています。",
    tests: "Vega-Lite SVG: 3 line series with point markers on a UTC temporal axis formatted in Japanese ('%-m月'), Japanese legend title and labels, axis titles; all text in Noto Sans JP.",
    islands: [
      {
        renderer: "vega-lite",
        width: 640,
        height: 320,
        caption: "図1　店舗別の月次売上高（万円）",
        spec: {
          $schema: "https://vega.github.io/schema/vega-lite/v6.json",
          width: 640,
          height: 320,
          autosize: { type: "fit", contains: "padding" },
          padding: 4,
          config: { font: "Noto Sans JP", axis: { labelFontSize: 11, titleFontSize: 11, gridColor: "#e5e8eb" }, legend: { labelFontSize: 11, titleFontSize: 11 }, view: { stroke: null } },
          data: { values },
          mark: { type: "line", point: { size: 30 }, strokeWidth: 2 },
          encoding: {
            x: { field: "month", type: "temporal", timeUnit: "utcyearmonth", title: "月", axis: { format: "%-m月", labelAngle: 0, tickCount: 12 }, scale: { type: "utc" } },
            y: { field: "sales", type: "quantitative", title: "売上高（万円）", axis: { format: ",d" }, scale: { zero: false } },
            color: { field: "store", type: "nominal", title: "店舗", sort: stores, scale: { range: ["#2f6fb0", "#e07b39", "#3a9b6a"] } },
          },
        },
      },
    ],
    table: {
      head: ["店舗", "年間売上高（万円）", "最高月"],
      numeric: [false, true, false],
      rows: stores.map((store) => {
        const v = values.filter((x) => x.store === store);
        const best = v.reduce((a, b) => (b.sales > a.sales ? b : a));
        return [store, fmtInt(v.reduce((a, b) => a + b.sales, 0)), `${Number(best.month.slice(5, 7))}月`];
      }),
    },
  };
}

function vegaliteLayeredDenseJa() {
  const next = rng(909);
  const days = isoDays("2025-01-01", 365);
  // Two seasonal peaks, noise and a weekend dip; stays within 0..180.
  const orders = days.map((_, i) => Math.round((100 + 25 * Math.sin((4 * Math.PI * i) / 365) + (next() - 0.5) * 40) * ((i + 3) % 7 >= 5 ? 0.6 : 1.05)));
  const values = days.map((date, i) => ({ date, orders: orders[i] }));
  const target = 110;
  const total = orders.reduce((a, b) => a + b, 0);
  return {
    name: "vegalite-layered-dense-ja",
    lang: "ja",
    title: "日次受注件数と7日移動平均（2025年）",
    org: "株式会社みなと商事",
    headerMeta: ["受注管理レポート", "対象期間：2025年1月1日〜12月31日", "単位：件"],
    pageFont: PAGE_JA,
    fonts: ["Noto Sans 400", "Noto Sans JP 400", "Noto Sans JP 700"],
    intro: "日次の受注件数（棒）と7日移動平均（線）です。破線は目標の1日110件を示します。",
    tests: "Vega-Lite SVG, dense: 4 layers over 365 days (bars, a window-transform 7-day moving average line, a rule and a text annotation), UTC temporal axis with Japanese month format, Japanese titles; font list 'Noto Sans', 'Noto Sans JP' with per-glyph fallback.",
    islands: [
      {
        renderer: "vega-lite",
        width: 680,
        height: 344,
        caption: "図1　日次受注件数（棒）、7日移動平均（線）、目標（破線）",
        spec: {
          $schema: "https://vega.github.io/schema/vega-lite/v6.json",
          width: 680,
          height: 344,
          autosize: { type: "fit", contains: "padding" },
          padding: 4,
          config: { font: "Noto Sans, Noto Sans JP", axis: { labelFontSize: 10, titleFontSize: 11, gridColor: "#e5e8eb" }, view: { stroke: null } },
          data: { values },
          encoding: {
            x: { field: "date", type: "temporal", title: "日付", axis: { format: "%-m月", tickCount: 12, labelAngle: 0 }, scale: { type: "utc" } },
          },
          layer: [
            { mark: { type: "bar", color: "#b5cde8", width: 1.4 }, encoding: { y: { field: "orders", type: "quantitative", title: "受注件数（件）", scale: { domain: [0, 180] } } } },
            {
              transform: [{ window: [{ op: "mean", field: "orders", as: "avg7" }], frame: [-6, 0], sort: [{ field: "date" }] }],
              mark: { type: "line", color: "#2f6fb0", strokeWidth: 1.8 },
              encoding: { y: { field: "avg7", type: "quantitative" } },
            },
            { data: { values: [{ target }] }, mark: { type: "rule", color: "#c2455a", strokeDash: [6, 4], strokeWidth: 1.2 }, encoding: { x: null, y: { field: "target", type: "quantitative" } } },
            {
              data: { values: [{ target, label: `目標 ${target}件` }] },
              mark: { type: "text", align: "right", dx: -4, dy: -7, fontSize: 11, color: "#c2455a" },
              encoding: { x: { value: "width" }, y: { field: "target", type: "quantitative" }, text: { field: "label" } },
            },
          ],
        },
      },
    ],
    table: {
      head: ["指標", "値"],
      numeric: [false, true],
      rows: [
        ["年間受注件数", `${fmtInt(total)}件`],
        ["1日平均", `${round(total / days.length, 1).toFixed(1)}件`],
        ["最多日", `${Math.max(...orders)}件`],
        ["目標達成日数", `${orders.filter((o) => o >= target).length}日`],
      ],
    },
  };
}

function vegaBarSimpleLatin() {
  const next = rng(1010);
  const categories = ["Rent", "Salaries", "Software", "Travel", "Marketing", "Utilities", "Insurance"];
  const values = categories.map((category) => ({ category, amount: Math.round(2000 + next() * 38000) }));
  const font = "Noto Sans";
  return {
    name: "vega-bar-simple-latin",
    lang: "en",
    title: "Operating expenses by category, Q3 2026",
    org: "Northwind Trading GmbH",
    headerMeta: ["Expense report", "Period: Jul to Sep 2026", "Currency: EUR"],
    pageFont: PAGE_LATIN,
    fonts: ["Noto Sans 400", "Noto Sans 700"],
    intro: "Booked operating expenses for the third quarter, by cost category. The figure is a hand-written Vega spec (not Vega-Lite).",
    tests: "Vega (not Vega-Lite) SVG: band and linear scales, two axes with grid and titles, rect marks and text labels using a format() signal expression; fonts set per axis and mark; autosize fit.",
    islands: [
      {
        renderer: "vega",
        width: 640,
        height: 304,
        caption: "Figure 1. Operating expenses per category, Q3 2026 (EUR).",
        spec: {
          $schema: "https://vega.github.io/schema/vega/v6.json",
          width: 640,
          height: 304,
          padding: 4,
          autosize: { type: "fit", contains: "padding" },
          config: { axis: { labelFont: font, titleFont: font, labelFontSize: 10, titleFontSize: 11, gridColor: "#e5e8eb" }, text: { font } },
          data: [{ name: "table", values }],
          scales: [
            { name: "x", type: "band", domain: { data: "table", field: "category" }, range: "width", padding: 0.2, round: true },
            { name: "y", type: "linear", domain: { data: "table", field: "amount" }, nice: true, zero: true, range: "height" },
          ],
          axes: [
            { orient: "bottom", scale: "x", title: "Cost category" },
            { orient: "left", scale: "y", title: "Amount (EUR)", format: ",d", grid: true, tickCount: 6 },
          ],
          marks: [
            {
              type: "rect",
              from: { data: "table" },
              encode: {
                enter: {
                  x: { scale: "x", field: "category" },
                  width: { scale: "x", band: 1 },
                  y: { scale: "y", field: "amount" },
                  y2: { scale: "y", value: 0 },
                  fill: { value: "#2f6fb0" },
                },
              },
            },
            {
              type: "text",
              from: { data: "table" },
              encode: {
                enter: {
                  x: { scale: "x", field: "category", band: 0.5 },
                  y: { scale: "y", field: "amount", offset: -4 },
                  text: { signal: "format(datum.amount, ',d')" },
                  align: { value: "center" },
                  baseline: { value: "bottom" },
                  font: { value: font },
                  fontSize: { value: 10 },
                  fill: { value: "#1f2328" },
                },
              },
            },
          ],
        },
      },
    ],
    table: {
      head: ["Category", "Amount (EUR)"],
      numeric: [false, true],
      rows: [...values.map((v) => [v.category, fmtInt(v.amount)]), ["Total", fmtInt(values.reduce((a, b) => a + b.amount, 0))]],
    },
  };
}

function echartsTwoChartsLatin() {
  // Two ECharts islands on one page, each with its own gradients and clip
  // paths. Chrome renders every chart in one page, so ECharts gives them
  // distinct ids (zr0-…, zr1-…); a renderer that gave both charts the same ids
  // would make the second chart's url(#…) fills point at the first chart's
  // gradients. The two charts use different gradient colours, so that shows
  // up as wrong colours, not just as different markup.
  const next = rng(1101);
  const quarters = ["Q1", "Q2", "Q3", "Q4"];
  const regions = ["North", "South", "East", "West", "Central"];
  const margin = regions.map(() => Math.round(180 + next() * 220));
  const days = 120;
  const t0 = Date.UTC(2026, 6, 1);
  const balance = walk(next, days, 42000, 60, 1800);
  const gradient = (top, bottom) => ({ type: "linear", x: 0, y: 0, x2: 0, y2: 1, colorStops: [{ offset: 0, color: top }, { offset: 1, color: bottom }] });
  return {
    name: "echarts-two-charts-latin",
    lang: "en",
    title: "Gross margin and cash balance, H2 2026",
    org: "Northwind Trading GmbH",
    headerMeta: ["Finance report", "Period: Jul to Oct 2026", "Currency: EUR"],
    pageFont: PAGE_LATIN,
    fonts: ["Noto Sans 400", "Noto Sans 700"],
    intro: "Gross margin per region for the half year, and the daily cash balance over the same months. Both charts sit on one page.",
    tests: "Two ECharts SVG islands on one page, each with linear gradients (bars; area under a line) and clip paths (the line series and its area are clipped to the grid); the two charts' gradients differ in colour, so ids shared between islands would show as wrong colours. Latin text.",
    islands: [
      {
        renderer: "echarts",
        width: 640,
        height: 288,
        caption: "Figure 1. Gross margin per region (thousand EUR), bars with a vertical gradient.",
        spec: {
          textStyle: { fontFamily: LATIN },
          title: { text: "Gross margin by region (kEUR)", left: 0, top: 0, textStyle: { fontFamily: LATIN, fontSize: 13, fontWeight: "bold" } },
          grid: { left: 56, right: 16, top: 40, bottom: 28 },
          xAxis: { type: "category", data: regions, axisTick: { alignWithLabel: true } },
          yAxis: { type: "value", splitLine: { lineStyle: { color: "#e5e8eb" } } },
          series: [
            {
              type: "bar",
              name: "Gross margin",
              data: margin,
              barWidth: "56%",
              itemStyle: { color: gradient("#2f6fb0", "#9db7d5"), borderRadius: [3, 3, 0, 0] },
              label: { show: true, position: "top", fontSize: 10, fontFamily: LATIN },
            },
          ],
        },
      },
      {
        renderer: "echarts",
        width: 640,
        height: 288,
        caption: `Figure 2. Daily cash balance (EUR), ${quarters[2]} and the start of ${quarters[3]}, area with a vertical gradient.`,
        spec: {
          useUTC: true,
          textStyle: { fontFamily: LATIN },
          title: { text: "Cash balance (EUR)", left: 0, top: 0, textStyle: { fontFamily: LATIN, fontSize: 13, fontWeight: "bold" } },
          grid: { left: 64, right: 16, top: 40, bottom: 28 },
          xAxis: { type: "time", min: t0, max: t0 + (days - 1) * 86400000, splitNumber: 5, axisLabel: { fontSize: 10, formatter: "{d} {MMM}", hideOverlap: true } },
          yAxis: { type: "value", min: 30000, splitLine: { lineStyle: { color: "#e5e8eb" } } },
          series: [
            {
              type: "line",
              name: "Balance",
              data: balance.map((v, i) => [t0 + i * 86400000, v]),
              showSymbol: false,
              clip: true,
              lineStyle: { width: 1.5, color: "#c2455a" },
              itemStyle: { color: "#c2455a" },
              // opacity 1: Chrome's PDF output drops fill-opacity on gradient
              // fills (its screen rendering and fulgur apply it; docs/COMPAT.md).
              areaStyle: { color: gradient("#e07b39", "#fbe3d3"), opacity: 1, origin: "start" },
            },
          ],
        },
      },
    ],
  };
}

export const fixtures = [
  echartsBarSimpleLatin(),
  echartsLineDenseLatin(),
  echartsBarSimpleJa(),
  echartsPieJa(),
  echartsMixedDenseJa(),
  vegaliteBarSimpleLatin(),
  vegaliteScatterDenseLatin(),
  vegaliteLineJa(),
  vegaliteLayeredDenseJa(),
  vegaBarSimpleLatin(),
  echartsTwoChartsLatin(),
];
