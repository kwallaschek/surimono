# fulgur 0.40.0 and fulgur-raikiri 0.41.0: media queries when printing

**Project:** fulgur 0.40.0 (blitz-dom 0.2.4) and fulgur-raikiri 0.41.0 · **Status:**
drafts for Kay to file (one issue each) · **Local patch:** none; surimono's
printer-safe output resolves the media type itself where it can do so without
changing what Chromium prints (`crates/surimono/src/print/safe/media.rs`, #200).

Found by Q-PKG-03 (16 Nov 2026) with one probe page per question, each printed by
Chromium 153.0.8010.12 (`page.pdf`, `preferCSSPageSize`), fulgur 0.40.0 and
fulgur-raikiri 0.41.0. The pages and what each printer printed are in
`crates/surimono/tests/data/print-media/` (`*.html`, `*.measured.json`;
`measure.py` reproduces them). These are behaviour differences from a browser's
print, not defects with a security or cost side.

| | Chromium 153 | fulgur 0.40.0 | fulgur-raikiri 0.41.0 |
|---|---|---|---|
| Media type | `print` | `screen` | `print` |
| `<style media>` | honoured | ignored: every `<style>` applies | honoured |
| `<link media>`, `@import` media | honoured | honoured, as `screen` | honoured |
| `width` | 816 px (the tool's paper: Playwright's Letter, whatever `@page` says) | 680 px (the page area of A4 with 15 mm margins) | about 480 px |
| `height` | 1056 px | no value matches | no value matches |
| `prefers-color-scheme` | `light` | `light` | neither value |
| `orientation`, `color`, `hover`, `pointer` | portrait, yes, `hover`, `fine` | none match | none match |
| `resolution` | `min-resolution: 2dppx` matches | 1dppx | none match |
| `@media { … }` (empty list) | applies | applies | doesn't apply |

## 1. blitz-dom 0.2.4: the device is always `screen`, and `<style media>` is ignored

Reproducer:

```html
<style media=screen>p{color:red}</style>
<style>@media print{p{color:blue}}</style>
<p>x</p>
```

Chromium 153 prints blue; fulgur 0.40.0 prints red. Two causes, as read in the
published crate (`document.rs`): `make_device` builds the device with
`MediaType::screen()`, so `@media print` never applies to a PDF and
`@media screen` always does; and `process_style_element` passes
`MediaList::empty()` to `make_stylesheet`, so a `<style>`'s `media` attribute is
never looked at (HTML, "The style element": the attribute says which media the
styles apply to).

Proposal: parse the `media` attribute into the sheet's `MediaList`, as is done
for `<link>`; let the embedder choose the media type, and have fulgur choose
`print` (Media Queries 4 §2.3: `print` matches printers and devices intended to
reproduce a printed display, such as a browser's print preview).

## 2. fulgur 0.40.0: media `width` is the page area, and other features match nothing

`blitz_adapter.rs` passes the content width as the viewport (about line 131),
so on A4 with 15 mm margins `(min-width: 681px)` is false. Media Queries 4 §4.1:
for paged media `width` is the width of the page box; Chromium answers with the
full paper width. `height`, `orientation`, `color`, `hover` and `pointer` match
no value, and `resolution` is 1dppx. Reproducer: `features.html` in the matrix.

Proposal: answer `width` and `height` with the page box, `orientation` from it,
and give `color` a value for a colour PDF.

## 3. fulgur-raikiri 0.41.0: an empty media query list is false; features match nothing

`@media { p { color: blue } }` doesn't apply (`types.html`, label `empty-list`).
Media Queries 4: an empty media query list evaluates to true; Chromium
applies it. No `prefers-color-scheme` value matches, `min-width` stops matching
near 480 px, and `height`, `orientation`, `color`, `hover`, `pointer` and
`resolution` match nothing. Measured only; Raikiri's source wasn't read.

## What surimono does meanwhile

Its printer-safe output rewrites each query the media type decides so that it
gives under any type what it gives under `print`, in the `<style>` sheets of a
page whose CSS it has all read. That covers item 1 for inline and inlined
sheets; a page with a stylesheet `<link>` or an `@import` it wasn't given is left
as it was. Features (items 2 and 3) are left for the printer.
