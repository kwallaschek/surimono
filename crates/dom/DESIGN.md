# surimono-dom design notes

Phase-2 scaffolding. Nothing here is public API: the crate is `publish = false`
and the facade doesn't re-export it. This note records the P1-DOM-02 spike
(id-based handles from rquickjs to blitz-dom 0.2.4) and what E2-02 should keep
or change.

## Handles, not pointers

JavaScript never holds a pointer, an rquickjs class instance or a blitz node id.
A node wrapper is a plain object; its `[generation, handle]` pair sits in a
`WeakMap` private to the bindings' closure (`src/glue.js`), so a script can
neither read nor forge it. Every accessor passes the pair to a native function,
which resolves it through the loaded document on that call:

```
JS wrapper ──WeakMap──▶ [generation, handle] ──native call──▶ Document::resolve
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
- **Identity**: the bindings cache one wrapper per handle (a `Map`), so
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
own limits (`DomLimits`): `max_nodes` (default 100,000) and `max_depth`
(default 512, checked on insertion; bounds blitz-dom's recursive subtree walks,
which could otherwise overflow the stack). Both are fatal faults. Text sizes are
bounded only indirectly (the string must first exist in the JS heap); E2-02
should charge text bytes to a document budget too.

## Measured

`tests/soak.rs`, the islands soak method (live heap bytes through a counting
global allocator; RSS printed). Each render: reset, parse a 40-row page, wrap
every row five times, create and drop 3 × 201 nodes, drop wrapped nodes, and
every 50th render use a stale wrapper. Debug build, 300 renders: live heap
772,734 → 772,830 bytes after warm-up (+96 B, limit +64 KiB), about 12 ms per
render. Release, 2,000 renders (`--ignored soak`, run by `scripts/ci/soak.sh`):
762,499 → 762,595 bytes from render 500 to 2,000 (+96 B, one step between
renders 1,250 and 1,500, the size of a container growing once), resident
10,268 → 10,272 KiB, about 7.6 ms per render.

## Deviations and gaps for E2-02

- **Removal drops nodes.** In the DOM a removed node lives on while referenced
  and can be reinserted. Here the `textContent` setter drops the old children
  (blitz `remove_and_drop_all_children`) and their wrappers fail with
  `NodeRemoved`. E2-02 needs detached-but-alive nodes (blitz `remove_node`
  keeps them in the slab) with their lifetime tied to the document, all freed
  when the document drops; the handle table already supports that.
- **Serialization.** blitz-dom 0.2.4's `outer_html` doesn't escape text
  (`<` stays `<`), so `Document::to_html` is test-only output. E2-02 needs a
  serializer per the HTML spec's fragment serialization algorithm.
- **Comments** carry no data in blitz-dom 0.2.4 (`NodeData::Comment`), so a
  comment's `textContent` is always empty.
- The prototypes are hand-written for the spike; E2-01 generates them from the
  WebIDL in `crates/webidl/`.
