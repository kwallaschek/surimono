# Compatibility

Generated from test results by the conformance researcher; do not edit by hand
once the generator exists (P1-CF-07). Every known failure is listed here with
the test that proves it. Expectations are never edited to match our output.

## Phase 1: islands

| Area | Status | Evidence |
|---|---|---|
| ECharts SVG renderer | not started | fixtures/islands/echarts-* |
| Vega and Vega-Lite | not started | fixtures/islands/vega-* |
| Mermaid | not started (stretch) | |
| Hostile scripts | not started | fixtures/hostile/ |

## Known gaps

| Gap | Evidence | Effect |
|---|---|---|
| `toLocaleString`, `toLocaleDateString` and the other `toLocale*String` methods ignore the locale argument in the sandbox (QuickJS has no Intl data): `new Date(Date.UTC(2026, 0, 1)).toLocaleDateString("ja-JP")` gives `"1/1/2026"`; Chrome gives `"2026/1/1"` | Reported by the reviewer of the core sandbox (p1-rt-05); Chrome's output checked in Chromium 153.0.8010.12 (time zone UTC): `"2026/1/1"`, `"2026/1/1 0:00:00"` for `toLocaleString("ja-JP")`, and `"1.234.567,5"` for `(1234567.5).toLocaleString("de-DE")` | Japanese (or any non-US) chart labels that format dates or numbers through `toLocale*String` come out in US format. The checkpoint-2 fixtures format through ECharts and Vega format strings instead, so they don't hit it |

## Printing gaps in fulgur 0.40.0 seen by the fixtures

These are fulgur's (or Blitz's), not surimono's. The fixture page template avoids
them so page diffs measure the islands (fixtures/README.md). Repros were made with
`fulgur render` and inspected in the PDF content stream.

| Gap | Repro | Effect |
|---|---|---|
| A single-side border ignores its colour and prints in `currentColor`: `border-bottom: 1px solid #d0d7de` (or with `border-bottom-color`) prints black on black text; `border: 1px solid #d0d7de` on all sides prints the right colour | `<div style="border-bottom: 2px solid red; height: 10px">` strokes `0 0 0 RG` | Table row rules and header underlines print black instead of light grey |
| A right-aligned flex item wraps earlier than in Chrome | `header { display: flex; justify-content: space-between; align-items: flex-end }` with a bold 12pt left item and a 9pt right item of three `<br>`-separated lines: fulgur wraps "Revenue report" onto two lines | Everything below moves down by a line, so the whole-page diff fails |
| Table column widths differ from Chrome's, with or without `table-layout: fixed` and `<col style="width: …">` | A 100%-wide table of 3 or 4 columns with right-aligned numbers: fulgur's numeric columns sit tens of pixels from Chrome's; a header that wraps in one engine does not in the other | Every row of the table differs |
| Fractional line heights accumulate differently | `font-size: 10pt; line-height: 1.4` plus mm margins above a chart: the chart lands about 1 px lower in fulgur | Thin chart lines (axes) fall on another pixel row |
| Line boxes do not grow for fallback glyphs | Latin-first font list (`"Noto Sans", "Noto Sans JP"`) on a paragraph containing Japanese: Chrome's line box takes Noto Sans JP's taller metrics, fulgur's does not | Content below moves by a pixel or more |
| No `text-spacing-trim` | Chrome (since 123) trims the spacing between adjacent full-width punctuation such as "）、" by default; fulgur prints full widths | Japanese captions and labels with that punctuation run longer in fulgur |
| SVG text: per-glyph fallback picks the Bold face | An SVG `<text font-family="Noto Sans, Noto Sans JP">10月</text>` (normal weight) with both Noto Sans JP faces available: `pdffonts` on fulgur's PDF of vegalite-layered-dense-ja's island lists only NotoSans-Regular and **NotoSansJP-Bold**; Chrome's lists NotoSansJP-Regular for the same labels. Same in echarts-mixed-dense-ja. When the family list names Noto Sans JP first (vegalite-line-ja), fulgur picks Regular | Japanese glyphs reached through fallback print heavier. This is the remaining fulgur floor on the fixtures: 4.5% (page) and 3.9% (island) on vegalite-layered-dense-ja, 2.5% and 2.9% on echarts-mixed-dense-ja, all within the 10% gate |

## Printing differences in Chrome seen by the fixtures

| Difference | Repro | Effect |
|---|---|---|
| Chrome rounds `@page` sizes to 1/300 inch | An island page with `@page { size: 640px 340px }`: `pdfinfo` on Chrome's PDF gives 255.12 pt tall instead of 255 pt (380 px gives 285.12 pt). pdftoppm at 96 dpi then rasterises 341 rows where fulgur's exact 255 pt gives 340 | Page sizes differ by one row and the comparison fails on size alone. The fixtures use island sizes in multiples of 8 px (1/12 inch), which are exact; `gen-islands.mjs` refuses others |
