# fulgur (Ruby binding): font bytes and a system-fonts switch

**Project:** fulgur, Ruby gem 0.40.0 · **Status:** draft for Kay to file · **Local patch:** none needed yet

## Problem
The Ruby binding exposes `AssetBundle#add_font_file` only. It has no way to pass
font data from memory and no `system_fonts:` option, although the Rust crate can
turn system fonts off. For reproducible output a caller needs both: the exact font
files it measured with (often loaded from a bundle or a database), and a guarantee
that no host font is picked up.

## Proposal
- `AssetBundle#add_font_bytes(String data)`, mirroring `add_font_file`.
- `system_fonts: false` on the engine or render options, mapping to the Rust
  option that already exists.

## Why it matters
surimono measures chart labels with the same font files fulgur prints with
(Parley 0.6 on both sides); with system fonts on, a host font can win the match
and printed text drifts from measured text. Evidence: surimono
`crates/gfx/tests/fulgur_layout.rs` (advance laid out by Blitz equals the measured
advance to 0.0000 px when both use the same files and system fonts are off).
