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

None recorded yet in surimono.

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
