# surimono-dom design notes

Phase-2 scaffolding. Nothing here is public API: the crate is `publish = false`
and the facade doesn't re-export it. This note records the P1-DOM-02 spike
(id-based handles from rquickjs to blitz-dom 0.2.4) and what E2-02 should keep
or change.

## Handles, not pointers

JavaScript never holds a pointer, an rquickjs class instance or a blitz node id.
A node wrapper is a plain object carrying a record `{ g, h, m }` (generation,
handle, interface bits) in a private class field (`#s`, stamped on by a
closure-private class in `src/glue_head.js`); members brand-check with
`#s in o` and the interface bit. Private fields can't
be read, written or intercepted through any prototype, Proxy or reflection, so
a script can neither read nor forge a pair. Every accessor passes the pair to
a native function, which resolves it through the loaded document on that call:

```
JS wrapper ──#g/#h──▶ [generation, handle] ──native call──▶ Document::resolve
                                                                │ generation ≠ doc's → StaleDocument
                                                                │ handle retired     → NodeRemoved
                                                                ▼
                                                     blitz node id (slab slot)
```

- **Generation**: a process-wide counter, one value per parsed `Document`. A
  wrapper from another document can never match, even if the other document
  has the same shape (same node ids).
- **Handle**: a per-document number from `Document`'s handle table, never
  reused. Blitz-dom's node ids are slab slots and *are* reused after a drop,
  so exposing them would let a wrapper for a dropped node silently resolve to
  a new node. The table maps handle → node id (`None` once dropped) and node
  id → handle; it is dropped with the document.
- **Ownership**: the native functions hold an `Rc` to a slot (`Host`) that the
  document is moved into by `DomRuntime::load` and out of by `take`. No JS
  value keeps a document or node alive; dropping the `Document` frees the
  whole tree regardless of what the script still holds.
- **No call-time lookups.** After install the glue uses only intrinsics it
  captured then, null-prototype objects for records and caches, and natives
  that take and return primitives, never arrays (rquickjs fills an array with ordinary sets, which a
  script's `Array.prototype` index setters would intercept). A first version
  kept the pair in a `WeakMap` and called `WeakMap.prototype.get` at call
  time, so a script that replaced it could read and forge pairs (review of
  #80); `tests/hostile.rs` replaces those methods, `Map`, `Object`,
  `Function.prototype.call`/`apply`, `String` and the index accessors before
  and after the first wrapper exists.
- **Identity**: the bindings cache one wrapper per handle, so
  `getElementById('a') === getElementById('a')`. The cache is per generation
  and lives in the JS heap, under the sandbox's memory limit, and goes with a
  reset. E2-02 may switch to `WeakRef` values once the sandbox's intrinsics
  are settled; not needed while every render resets the context.

## Generated bindings

`crates/webidl/src/bindgen.rs` turns the `DOM_BINDINGS` table (interface and
member names, checked against the vendored IDL) into two checked-in files,
`src/generated/bindings.js` and `src/generated/dispatch.rs`; `tests/generated.rs`
fails when they are stale. Only listed members exist in JavaScript, so a page's
feature detection sees the truth. The hand-written glue is only infrastructure:
`glue_head.js` (brand, `wrap`, conversions, interface setup) and `glue_tail.js`.

- **One native.** Every member is `call(op, generation, handle, ...args)`. The
  glue applies the WebIDL conversions, so natives see primitives only;
  interfaces cross as handle numbers, 0 for null. `dispatch` is one `match` from
  `op` to a method of the generated traits, implemented on `Document` in
  `src/host.rs`. Installing costs one Rust function, not one per member.
- **Not generated.** Overloads, sequences and members with callback types make
  the generator fail with a typed error; dictionaries are ignored (`options`
  arguments). Add support when a listed member needs it.

## Parsing

`src/parse.rs` drives html5ever itself over blitz-html's `DocumentHtmlParser`,
adding: comment data (`NodeData::Comment` has no room, so `Document` keeps it in
a side table), the quirks mode, no `println!` of parse errors, and chunked
feeding that stops once the tree passes `max_nodes` or `max_depth` (html5ever
walks its open-element stack per start tag, quadratic on a deep tree inside one
uninterruptible call).

## Staying out of blitz-dom's document behaviour

blitz-dom is a browser engine's DOM: nodes joining the document load resources,
`<style>` text becomes stylesheets, form controls grow children. Here it is a
tree store, and those paths are where hostile input hurts it:

- `resolve_url` panics on a URL it can't resolve, for `<img src>` and `<link
  href>` as they join. With the default `data:` base that is every relative URL.
- `process_button_input` gives `<input type=button value=x>` a text child each
  time it joins, outside `max_nodes`.
- `clear_attribute` hits `unreachable!` for a `<link>`'s `href` unless a
  stylesheet was loaded.
- A `<style>` whose text has an `@import` with an unresolvable URL panics in
  `net.rs` (`import.url.url().unwrap()`), attached or not, because the mutator
  parses style text whenever a style element or its children change.

So, without patching blitz-dom (a `[patch]` wouldn't reach users of published
crates):

- The document node's `IS_IN_DOCUMENT` flag is cleared at parse. blitz-dom's
  hooks run when a node's flag differs from its new parent's, so none ever do.
- `link`'s `href` is removed directly, not through `clear_attribute`.
- Text under a `<style>` lives in `side_data`, with empty text nodes in blitz-dom,
  from the parse on and for any text joining a style element. `char_data` is the
  one reader. Which text is dangerous can't be told without parsing CSS, across
  node boundaries, so all of it stays out.

`to_html` writes style text out unchanged, so `<style>@import "http://[";</style>`
reaches the printing engine's blitz-dom, where the panic lives: the fulgur hand-off
(E2-08) has to deal with that before it goes live.

Phase 3 needs both back, for layout: flags set over the tree and screened CSS
given to blitz-dom, both done once before layout, not while scripts mutate.

## Objects other than nodes

The handle table holds `Object`s, not only nodes: live lists (`childNodes`,
`children`, `getElementsBy*`), static lists, `classList`, `attributes`,
`dataset`, `Attr`. Each is addressed through its owner node, which has a handle
and so stays alive; `[SameObject]` members look their handle up in a
`(node, key)` map. A live list is recomputed when the document's `version`
moved, with one cached result (bounded memory; a `for (i…) list[i]` loop isn't
quadratic). Static-list entries count against `8 × max_handles`, since they live
outside the JS heap limit.

Interfaces with indexed or named getters are generated as `exotic()` tables and
wrapped in a `Proxy` whose handler lives in the glue (WebIDL §3.9). Target and
proxy both carry the record in the private field, which proxy traps don't see.

## Selectors

`src/select.rs` matches element by element through `Guarded`, a delegating
`selectors::Element`, instead of calling blitz-dom's query in one go. One match
can be polynomial in a wide tree (`:not(:not(b ~ b) ~ b)`) and nothing could
interrupt it; `Guarded` counts navigation steps and answers "no such element"
once the deadline passes, so the match ends quickly. Bracket nesting over 32 is
a `SyntaxError`: the parser and matcher recurse per level. The nth-child and
`:has` caches live for the whole query.

## Errors

Using a stale wrapper throws into the script (so it stops where it is) and
records a fault; the call then fails with `DomError::StaleDocument`,
`NodeRemoved` or `NoDocument` **even if the script caught the exception**:
continuing would mean computing with data that no longer exists. The same
applies to the DOM resource limits (below). Ordinary DOM errors
(`HierarchyRequestError`, `InvalidCharacterError`) are DOMExceptions the script
may catch, as in a browser. Sandbox limits outrank faults: a script stopped by
the time, memory or stack limit reports that limit.

## Sandbox and limits

Every native's body runs under `Deadline::guard` through `guarded`, so a panic
ends the call with `HostPanic` however the script handles the exception, and
rquickjs doesn't leave it stored for a later call (#102). A panic may have
stopped a change to the tree halfway, so the document goes with the script's
state. `every_native_is_guarded` reads this file and fails on a native without it.

Every script runs through `surimono_core::Sandbox::run`, so the memory, time
and stack limits, determinism and the no-I/O rule apply unchanged; crates/dom
needed nothing new from core. Bindings are installed inside the first call on
a context (`glue.js` plus the natives) and again after the sandbox rebuilds
itself following a limit error or a reset. Every native checks the call's
`Deadline` first, as the islands' `measureText` does, so a loop of host calls
still ends with `TimeLimit`.

Nodes live in Rust, outside the JavaScript heap limit, so the document has its
own limits (`DomLimits`), all fatal faults:

- `max_nodes` (default 100,000), checked at parse and on every created node.
- `max_depth` (default 512, at most `DomLimits::MAX_DEPTH` = 1,024), checked
  at parse (html5ever doesn't cap nesting) and on every insertion. Our own
  walks (text content, serialization, subtree drops) are iterative; the bound
  is for blitz-dom's recursion when a subtree joins the document. The deepest
  allowed tree is tested on a 1 MiB thread (Puma's stack size).
- `max_handles` (default 1,000,000): handles are never reused, so a script
  that creates and drops nodes in a loop grows the handle table (16 bytes an
  entry) while the node count stays flat.
- `max_text_bytes` (default 256 MiB): text, comment data, element names, and
  attribute names and values (plus `ATTRIBUTE_COST` each) held by the document;
  names with their namespace and prefix, which a script can make any size. Charged before a
  write and credited on overwrite and drop. These strings are outside the JS heap
  limit, and one JS string can be stored on many nodes.

**Node lifetime.** A node with a handle lives until the document drops: a script
may still hold it, and the DOM keeps removed nodes usable (`removeChild` then
`appendChild`). A removed subtree with no handle in it can't be reached again and
is dropped at once, iteratively (`Document::remove_all_children`,
`drop_subtree`): blitz-dom's own subtree drop recurses per level, and removing
children one at a time rescans the parent's list. A script that keeps wrapping
and discarding nodes therefore counts them against `max_nodes` until the render
ends; wrappers have no weak references yet.

blitz-dom's insertion is only used for parentless nodes (`Document::place`): given
a node that already has a parent, it scans the old parent's list by node id after
adding to the new one, so a move within one parent would delete the node.

## Measured

`tests/soak.rs`, the islands soak method (live heap bytes through a counting
global allocator; RSS printed). Each render: reset, parse a 40-row page, wrap
every row five times, create and drop 3 × 201 nodes, drop wrapped nodes, and
every 50th render use a stale wrapper. Debug build, 300 renders: live heap
772,734 → 772,830 bytes after warm-up (+96 B, limit +64 KiB), about 12 ms per
render (first version). Release, 2,000 renders (`--ignored soak`, run by
`scripts/ci/soak.sh`), after the review fixes: 530,747 → 530,843 bytes from
render 500 to 2,000 (+96 B, one step, the size of a container growing once),
resident 10,068 KiB flat, about 4.8 ms per render.

The soak measures growth between renders. Growth within one render (handle
churn: create, append, `textContent = ''` in a loop) is bounded by
`max_handles` and tested in `tests/hostile.rs`.

## Deviations and gaps for E2-02

- **Serialization.** blitz-dom 0.2.4's `outer_html` doesn't escape text
  (`<` stays `<`) and recurses per level, so `Document::to_html` has its own
  iterative serializer (HTML §13.3): HTML, SVG and MathML elements, attribute
  names with their `xml:`/`xmlns:`/`xlink:` prefixes, raw text and void
  elements for HTML elements only, text and comments (`tests/serialize.rs`).
  It serializes with scripting disabled, because blitz-html 0.2.0 parses
  that way (`scripting_enabled: false`): `<noscript>` content is markup, so
  its text is escaped, and a reparse (fulgur's included) gives the same
  tree. E2-02 adds `<template>` contents.
