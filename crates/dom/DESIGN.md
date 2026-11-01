# surimono-dom design notes

Phase-2 scaffolding. Nothing here is public API: the crate is `publish = false`
and the facade doesn't re-export it. This note records the P1-DOM-02 spike
(id-based handles from rquickjs to blitz-dom 0.2.4) and what E2-02 should keep
or change.

## Handles, not pointers

JavaScript never holds a pointer, an rquickjs class instance or a blitz node id.
A node wrapper is a plain object carrying its `[generation, handle]` pair in
private class fields (`#g`, `#h`, stamped on by a closure-private class in
`src/glue.js`); the accessors brand-check with `#h in o`. Private fields can't
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
  captured then (`Object.create`, `Object.defineProperty`, `String`), a
  null-prototype object as cache, and natives that return numbers and
  strings, never arrays (rquickjs fills an array with ordinary sets, which a
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

Subtree drops don't use blitz-dom's recursive `remove_and_drop_all_children`
on deep trees: `Document::drop_children` cuts every grandchild and below loose
and drops it as a parentless leaf, then lets blitz drop the (now leaf)
children for its restyle bookkeeping. Each node is visited once, so a wide
tree is linear too. Text sizes are bounded only indirectly (the string must
first exist in the JS heap); E2-02 should charge text bytes to a document
budget too.

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

- **Removal drops nodes.** In the DOM a removed node lives on while referenced
  and can be reinserted. Here the `textContent` setter drops the old children
  (blitz `remove_and_drop_all_children`) and their wrappers fail with
  `NodeRemoved`. E2-02 needs detached-but-alive nodes (blitz `remove_node`
  keeps them in the slab) with their lifetime tied to the document, all freed
  when the document drops; the handle table already supports that.
- **Serialization.** blitz-dom 0.2.4's `outer_html` doesn't escape text
  (`<` stays `<`) and recurses per level, so `Document::to_html` has its own
  iterative serializer (HTML §13.3): HTML, SVG and MathML elements, attribute
  names with their `xml:`/`xmlns:`/`xlink:` prefixes, raw text and void
  elements for HTML elements only, text and comments (`tests/serialize.rs`).
  It serializes with scripting disabled, because blitz-html 0.2.0 parses
  that way (`scripting_enabled: false`): `<noscript>` content is markup, so
  its text is escaped, and a reparse (fulgur's included) gives the same
  tree. E2-02 adds `<template>` contents.
- **Comments** carry no data in blitz-dom 0.2.4 (`NodeData::Comment`), so a
  comment's `textContent` is always empty and it serializes as `<!---->`
  (ignored spec test `comments_keep_their_data`).
- **Parse errors on stdout.** blitz-html 0.2.0's `TreeSink::finish` prints
  every html5ever parse error as `ERROR: <message>` with `println!` (e.g.
  `ERROR: Unexpected token` for a document without `<!doctype html>`).
  libtest captures it unless `--nocapture` (the soak script) or a test
  fails. Fixing it needs a `[patch]` of blitz-html or our own `TreeSink`.
- The prototypes are hand-written for the spike; E2-01 generates them from the
  WebIDL in `crates/webidl/`.
