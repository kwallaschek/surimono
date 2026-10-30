# fulgur 0.40.0: layout gaps found by surimono's fixtures

**Project:** fulgur 0.40.0 (Blitz 0.2.4) · **Status:** drafts for Kay to file (one issue each) · **Local patch:** none; surimono's fixture template avoids them

Each item was found by printing the same page with Chrome 153 and fulgur 0.40.0 and
comparing rasters (surimono `fixtures/tools/compare.sh`). The repro pages and scores
are in surimono's `docs/COMPAT.md` (section "fulgur 0.40.0") and
`fixtures/islands/README.md`.

Exact repro HTML, observed vs expected output and scores for items 1 to 5 are in
`docs/COMPAT.md`; copy them into each issue rather than paraphrasing.

1. **Single-side border colour** (e.g. `border-bottom-color` on one side) prints
   differently from Chrome.
2. **`flex-wrap`** rows lay out differently from Chrome.
3. **Table column widths** differ from Chrome's auto table layout.
4. **Fractional line heights and fallback line boxes** differ from Chrome.
5. **`text-spacing-trim`** behaves differently from Chrome for CJK punctuation; the
   fixtures set `text-spacing-trim: space-all` to avoid it.
6. **Bold fallback for Japanese in inline SVG text:** normal-weight Japanese in an
   inline SVG prints in Noto Sans JP **Bold** when both weights are bundled
   (`pdffonts` shows the Bold face). Root cause is the usvg path using system fonts
   (see `blitz-svg-document-fonts.md`); listed here because it shows up in fulgur
   output.
7. **`@page` size rounding:** Chrome rounds page sizes to 1/300 inch, fulgur prints
   exact sizes; not a fulgur bug, noted so maintainers aren't surprised by 1 px
   differences in comparisons.

For each issue: attach the fixture HTML and fonts from surimono's repo, the two PNGs,
and the score from compare.sh.
