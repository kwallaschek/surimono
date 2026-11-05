# Draft: an entry point that prints a document already built

For Kay to propose on fulgur. Not filed. Seen in fulgur 0.40.0.

## Problem

surimono runs a page's scripts on blitz-dom and hands fulgur the result. Today that means serializing the tree and letting fulgur parse it again. Parsing twice costs time and memory, and the tree the scripts saw isn't the tree that is printed. Layout queries in surimono's phase 3 need both to be the same tree, or a script's measurement won't match the page.

`Engine::render(&self, html: &str)` is the only way in. `layout_to_drawables` takes `&str` and parses inside `blitz_adapter::parse_inner`, which builds the `HtmlDocument` (viewport, fonts, base URL, net provider) itself. It also runs twice when `needs_pass_two` is set, parsing the HTML again with the first pass's anchor map, so a single prepared document can't serve that path as things stand.

## Proposal

Add an entry point that takes a prepared `HtmlDocument`, or a closure that builds one:

```rust
impl Engine {
    /// `build` is called once per layout pass (once or twice).
    pub fn render_document(&self, build: impl FnMut() -> HtmlDocument) -> Result<Vec<u8>>;
}
```

The closure form keeps the two-pass path: passes mutate the document, so each needs its own. A caller that can clone or rebuild its tree supplies one per pass. fulgur would still apply its DOM passes, and the caller would have to build the document with the viewport and fonts fulgur uses, so `Engine` should expose those (or a `DocumentConfig` it prepared).

## What the caller has to do first

blitz-dom's hooks are bypassed while scripts run, so before the hand-off surimono must, once and between elements: flush each element's `style` attribute (the no-namespace one), give blitz-dom screened `<style>` text, set the in-document flags, and drop `<noscript>`, since fulgur parses with scripting off.

## Status in surimono

Not implemented. Output is serialized HTML (docs/SPEC.md), screened for the printer in `crates/surimono/src/print.rs`.
