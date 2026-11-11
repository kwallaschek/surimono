# parley 0.6.0: two gaps that reach fulgur 0.40.0's PDFs

**Project:** parley 0.6.0 (linebender; the version blitz-dom 0.2.4 and so fulgur 0.40.0 and 0.41.0 pin) · **Status:** P1 is fixed in parley 0.12.0, so nothing to file there; it is written up because the fix has not reached fulgur. P2 is a feature request draft for Kay · **Local patch:** none in surimono's workspace; P1 was proved in a scratch build of fulgur-cli 0.40.0

Found by Q-GFX-01 (11 Nov 2026). "Checked" means the published crate source was
read.

## P1. Max-content width forgets every line before the last forced break

```html
<style>header { display: flex; justify-content: space-between; align-items: flex-end }</style>
<header><div><b>Revenue report</b></div>
<div style="text-align:right">Fiscal year 2026<br>Prepared 1 November<br>Page 1</div></header>
```

**Chrome:** the right item is as wide as its widest line, "Prepared 1
November", and has three lines. CSS Sizing 3: the max-content size is the
size of the box when no soft wrap opportunity is taken; a forced break still
ends a line, so the width is the widest of the forced lines.

**fulgur 0.40.0:** the right item is as wide as "November", its longest word,
and has six lines: "Fiscal year / 2026 / Prepared / 1 / November / Page 1"
(`pdftotext -bbox`: the item is 44.5 pt wide). Every shrink-to-fit box whose
text has a `<br>` or a preserved newline is affected: flex and grid items,
inline-blocks, floats, table cells, absolutely positioned boxes. This is the
"right-aligned flex item wraps earlier than in Chrome" row of surimono's
`docs/COMPAT.md`.

**Root cause.** `LayoutData::calculate_content_widths`
(parley 0.6.0 `src/layout/data.rs:509-568`) keeps `running_max_width` and
folds it into `max_width` only once per item, after the item's clusters
(`:560-561`). At a mandatory boundary it resets the running width first
(`:540-542`):

```rust
if boundary == Boundary::Mandatory {
    running_max_width = 0.0;
}
```

So within one text run only the text after the last forced break counts. For
the reproducer `max` is the width of "Page 1", less than `min` (the width of
"November", 59 px). blitz-dom hands taffy that max-content width, taffy floors
the flex item at its min-content width, and the box is one word wide.

**Smallest fix** (2 lines; trivial, low risk):

```diff
@@ parley 0.6.0 src/layout/data.rs:540
                             if boundary == Boundary::Mandatory {
+                                // The line that ends here counts too.
+                                max_width = max_width.max(running_max_width - trailing_whitespace);
                                 running_max_width = 0.0;
                             }
```

**Newer releases.** Fixed: parley 0.12.0 replaces the function with a
`ContentWidthsMeasurer` whose `hard_break` folds the line into both widths
(`src/layout/data.rs:399-480`). The version between 0.7.0 and 0.12.0 that
introduced it was not looked up. blitz-dom 0.3.0-beta.4 depends on parley 0.12;
blitz-dom 0.2.4 on 0.6, and cargo will not pick a 0.6.x that does not exist. A
fulgur user gets the fix when fulgur moves to Blitz 0.3, or if fulgur carries a
`[patch]` of parley 0.6.

**Workaround without touching the printer.** None that is neutral in Chrome.
An explicit width needs Chrome's max-content width and stops the box from
shrinking when there is less room; a `<div>` per forced line instead of `<br>`
changes what selectors match and puts a block inside an inline parent. `white-space: nowrap` does not help
(blitz-dom 0.2.4 ignores it, B4 in `blitz-print-gaps-0.2.4.md`).

**Proof.** fulgur-cli 0.40.0 rebuilt with the two lines prints the reproducer
with three lines in the right item, as Chrome does (scores and method in
`fulgur-layout-gaps-0.40.md`, "Scores").

## P2. No `text-spacing-trim`

```html
<p style="font: 20px 'Noto Sans JP'">（税込）、「合計」。</p>
```

**Chrome** (since 123): `text-spacing-trim: normal` is the initial value
(CSS Text 4, `text-spacing-trim`). It sets the full-width opening bracket at the line start
flush, and halves the advance of a closing punctuation mark that is followed by
another full-width punctuation mark ("）、" and "」。"), using the font's
`halt`/`chws` data. The sample is about 1.5 em shorter than the sum of its
advances.

**fulgur 0.40.0:** every glyph keeps its full advance. `text-spacing-trim:
space-all` in the page makes Chrome print the same widths, which is what
surimono's fixtures do; it does not help a user who wants Chrome's default look.

**Root cause.** Not implemented anywhere in the stack: stylo 0.8.0 (servo
build) has no `text-spacing-trim` property, parley 0.6.0 has no such style
property, and blitz-dom 0.2.4 passes `font_features: &[]`
(`src/stylo_to_parley.rs:182`), so a page cannot ask for a font feature with
`font-feature-settings` either.

**Fix size.** Deep: the trimming is contextual (which mark follows which, and
where the line starts), so it belongs in the shaper's caller, with the property
in stylo and Blitz. A font feature alone does not give it: Noto Sans JP 2.004
has `halt` and `palt` but no `chws`, and `halt` applied to a whole run halves
every full-width mark, not only the ones Chrome trims.

**Newer releases.** parley 0.12.0 has no text-spacing-trim (searched the source
for `spacing_trim`, `halt`, `chws`: no match). blitz-dom 0.3.0-beta.4 does pass
`font-feature-settings` to parley (`src/stylo_to_parley.rs:274`, `:494`).

**Workaround without touching the printer.** None that changes fulgur's
output. Documented instead: a page that must print the same in both sets
`text-spacing-trim: space-all`.
