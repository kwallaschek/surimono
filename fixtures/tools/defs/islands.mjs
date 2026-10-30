// Island fixture definitions. Each entry becomes fixtures/islands/<name>/.
// Fonts: every spec names its fonts explicitly; nothing may fall back to a
// system font. Vega and Vega-Lite specs fit the marker box exactly
// (autosize fit, contains padding), which the Chrome harness asserts.

import { fmtInt, rng } from "./data.mjs";

const LATIN = "'Noto Sans'";
const PAGE_LATIN = '"Noto Sans", "Noto Sans JP"';

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
        height: 300,
        caption: "Figure 1. Open invoice total per customer (EUR).",
        spec: {
          $schema: "https://vega.github.io/schema/vega-lite/v6.json",
          width: 640,
          height: 300,
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

export const fixtures = [echartsBarSimpleLatin(), vegaliteBarSimpleLatin()];
