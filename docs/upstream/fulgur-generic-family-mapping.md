# fulgur: map generic font families to bundled fonts

**Project:** fulgur 0.40.0 · **Status:** draft for Kay to file · **Local patch:** none

## Problem
With system fonts off, `font-family: sans-serif` (or any generic family) resolves
to no font and the text lays out at zero width, so it prints blank. Templates
commonly end their stacks with a generic family, and many use only a generic.

## Proposal
An option to map each CSS generic family (sans-serif, serif, monospace, and
optionally system-ui, cursive, fantasy) to one or more registered families, e.g.
`generic_family(GenericFamily::SansSerif, &["Noto Sans", "Noto Sans JP"])`, and in
Ruby `generic_families: { sans_serif: ["Noto Sans", "Noto Sans JP"] }`. Fontique
already supports generic family mapping on a collection.

## Evidence
surimono's `crates/gfx` tests lay text out with Blitz configured as fulgur does
and system fonts off: a generic family yields width 0.
