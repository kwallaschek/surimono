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
                                                                │ unknown handle     → StaleDocument
                                                                ▼
                                                     blitz node id (slab slot)
```

- **Generation**: a process-wide counter, one value per parsed `Document`. A
  wrapper from another document can never match, even if the other document
  has the same shape (same node ids).
- **Handle**: a per-document number from `Document`'s handle table, never
  reused. Blitz-dom's node ids are slab slots and *are* reused after a drop,
  so exposing them would let a wrapper for a dropped node silently resolve to
  a new node. The table maps handle → node id and node id → handle; a node
  with a handle lives as long as the document, so an entry never goes stale,
  and the table is dropped with the document.
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

- `style` attributes stay away from stylo (see Sandbox and limits); the tree has
  the text, and elements have no `style_attribute` block.

Phase 3 needs all three back, for layout: flags set over the tree, screened CSS
given to blitz-dom and `flush_style_attribute` per element, all done once before
layout, not while scripts mutate.

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
interrupt it; `Guarded` charges a clock for work (each element reached for the
selector's byte length, which also counts what `:is()` nests, each sibling
scanned, children `:empty` reads, attributes tested) and answers "no such element" once the deadline passes, so the match
ends quickly. blitz-dom's sibling lookups scan the parent per step, so `Guarded`
has its own, which search out from the last index found. The parser and matcher
recurse per bracket level and the matcher per combinator, on the stack, so
`parse_selectors` caps both, and the length (see the gaps below). The
nth-child cache lives for the whole query.

## Inline style

`element.style` is a `CSSStyleDeclaration` over the `style` attribute's text
(`src/style_attr.rs`); each read parses the attribute, so `setAttribute` stays
live and nothing is kept per element.

- **Names.** A property is supported if it is in `src/css_properties.rs` (the 822
  names in webref's `ed/css`, generated by `fetch-css-properties.sh` at the same
  commit as the IDL) or stylo knows it. Stylo alone isn't enough: in its Servo
  configuration it has no `fill`, `stroke` or `break-inside`, and the printer's SVG
  code reads those from the attribute. Legacy aliases map to their target. A
  custom name can be any text, as in Chrome; the serializer escapes it.
- **Values.** cssparser tokenizes, so comments, strings and blocks follow CSS
  Syntax, and a value is written closed: it can't run into the next declaration.
  A property stylo knows has its value checked by stylo, the parser the printer
  uses, so what it refuses, the printer couldn't have used; other properties, and
  values over 64 KiB, are checked as tokens. A stored value stays as written,
  apart from comments, whitespace runs and string quotes: no computed forms
  (`#F00`), no shorthand expansion.
- **Bounds.** One attribute may have 4,096 declarations, 65,536 CSS tokens and
  8 MiB (`DomError::StyleLimit`, fatal, because a read that fails can't be
  answered). Everything `style_attr::fits` checks goes for `setAttribute`,
  `setProperty`, any namespace's `style`, `innerHTML` and the page's own parse.
- **Stylo.** A stylo parse can't be interrupted, and its cost has shapes tokens
  don't show, so blitz-dom doesn't get to parse this attribute: it would on every
  write, and on every element made, for a layout that doesn't run here.
  `write_attribute` and `clear_attribute` change the attribute list directly, and
  the parser and clones make elements with the attribute under a name no input
  can have (`style_attr::hide`). A `style` read gives stylo one declaration of at
  most 64 KiB at a time, with the deadline looked at in between, so an overrun is
  one such parse; `stylo_cost_by_property_and_shape` (ignored; run on a release
  build) times every property against 49 shapes at every size that fits.
- **What the printer gets.** `fits` also bounds a single attribute for the
  printer's stylo, and for layout in phase 3, which will flush these once,
  between elements. Two costs need more than tokens. A push of a property the
  block has walks the block, and `all` pushes every longhand, so 60 KB of
  custom properties and `all:inherit` took 11 s: `Budget::declare` charges
  longhands times block size, up to 2^26 steps. `grid-template-areas` and the
  shorthands tokenize strings again and intern each name, so a declaration of
  those has at most 4,096 cells, a 64 by 64 grid. A `var()` can carry such a
  string out of a custom property, which isn't charged, and `<style>` text isn't
  screened at all: E2-08 has to bound the printer's parse as a whole.
  Parsing is linear, looks at the deadline per declaration, and nests at most
  32 brackets, for the recursion here and in stylo's `calc()`.

CSSOM defines the per-property attributes (`style.fontSize`) in prose, so the
generator has a pseudo member for them: property names go to the host, others are
ordinary expandos. `getComputedStyle` is phase 3.

## Events and the page's life

Listeners are JavaScript values, so events live in the glue (`glue_events.js`),
as timers' handlers do in the prelude: in private records, under the memory
limit, with no Rust state. The generator still writes these interfaces from the
IDL (`JS_INTERFACES`): names, constants, brand checks and conversions come from
WebIDL; each member calls a hand-written `H.<Interface>_<member>`. Lists are
null-prototype objects, so `Array.prototype` setters see nothing.

The global object is the window: it is branded, inherits from
`Window.prototype`, and an undefined `this` means it (WebIDL §3.7.6).

`DomRuntime::run_page` queues every lifecycle step as a zero-delay timer before
any page script runs, so they keep their order ahead of the page's own timers,
each is a task with a microtask checkpoint after it, and all run in one
`run_until_settled` call under one time limit. The order is in its doc comment.
Being timers would make them a page's to cancel and start a script's timers one
nesting level high, so: `clearTimeout` and `clearInterval` are the glue's and
refuse the lifecycle's ids (HTML §13.2.7 has no way to cancel "the end"), and
each step does its work in a microtask of its task, where the loop's nesting
level is 0 again. The parser has finished before the first script runs, so a
script sees the whole tree: `document.write` has no meaning here (not bound),
and a script can't observe the parse. A `<script>` a script inserts doesn't run
yet.

Which scripts run is "prepare the script element" (HTML §4.12.1.1) with the
MIME Sniffing list of JavaScript essences, including `language`. Template
contents never run. The tree is parsed with scripting on, so `<noscript>` holds
raw text and no script. Departures: `nomodule` is ignored, since modules don't
run here and the fallback is what a page has for us; an `async` script keeps
its tree position, which the spec allows and which is deterministic.

**Reporting.** `reportException` is HTML §8.1.6.2, with its error reporting
mode: while an `error` event is being fired, a further exception is counted and
fires nothing, so a listener calling a missing function can't recurse. The
glue wraps `setTimeout`, `setInterval`, `requestAnimationFrame` and
`queueMicrotask` so their callbacks report the same way as scripts and
listeners, and reach `PageReport.errors`. A string handler runs through a native
for that reason. Messages are capped at 4 KiB, as the sandbox's are.

**Limits are not exceptions.** A thrown value is classified first, as
`exception_error` in crates/core does, so no listener sees or cancels it.
Memory and time are sticky in the sandbox, which stops the script itself; a
stack overflow isn't, and a `try` may catch one (as in a browser). An uncaught
one reaches `n.limit`, which records the error and raises the sandbox's cancel
flag, the only stop crates/dom can reach. The watchdog sees it within about 5 ms;
until the call returns natives refuse to work, and `finish_call` turns the
`Cancelled` back into the limit, or resets the sandbox if the call ended first.
The flag is private to `DomRuntime`: nothing else can cancel a `DomRuntime`'s
sandbox. A sticky stop in core, as for a panic, would remove that wait.

**Event handlers** (HTML §8.1.7): `handlers(id, names)` in `bindings.js` comes
from the IDL's `EventHandler` attributes, for HTMLElement, Document and Window.
A handler's state is in the target's record, next to its listeners, and
`activate` adds a listener of its own in the place the spec gives it.
Content attributes live in Rust, so `syncHandlers` compares them when an event
reaches the target, a listener is added or the handler is read, which are the
only points a page can observe a listener's place from. Raw text compiles once,
on first use. On a body or frameset element the Window-reflecting set is the
window's. `document.currentScript` is the glue's, set from a script's run until
a separate lifecycle step after its microtasks, which then fires its `load`.

**Gaps** (each for docs/COMPAT.md): no `window.event`; `isTrusted` is an
ordinary accessor, not [LegacyUnforgeable]; no microtask checkpoint between the
listeners of a browser-fired event (`L1,L2,micro`, the spec gives `L1,micro,L2`);
the handler attributes sit on `Window.prototype`, not on the window itself; a
handler's scope has no form owner; SVG and MathML elements have no `on*`
attributes (no interfaces bound), and `<svg><script>` doesn't run; the
WindowEventHandlers aren't readable on a body element, only reachable through
its content attributes; `AbortSignal` in listener options is ignored;
`unhandledrejection` doesn't fire. ErrorEvent `filename`, `lineno` and `colno` are always
empty and 0.

## Errors

Using a stale wrapper throws into the script (so it stops where it is) and
records a fault; the call then fails with `DomError::StaleDocument` or
`NoDocument` **even if the script caught the exception**: continuing would mean
computing with data that no longer exists. The same applies to the DOM resource
limits (below) and to `DomError::Unsupported`, for what the DOM defines and this
crate doesn't do yet, so a script can't take a refusal for the standard's
answer. Ordinary DOM errors
(`HierarchyRequestError`, `InvalidCharacterError`) are DOMExceptions the script
may catch, as in a browser. Sandbox limits outrank faults: a script stopped by
the time, memory or stack limit reports that limit.

A limit error ends the call wherever it surfaces, and the engine says which
errors are one (`surimono_core::limit_error`, a mark on the error object), so
a script can neither forge one nor hide one by renaming it. Thrown, it reaches
the glue's `reportException`, which asks Rust before any event fires. In an
async function or a promise reaction it rejects a promise instead: if nothing
handled that promise when the microtasks have drained, core's rejection tracker
ends the call, and handling it is catching it. Core's event loop has that
checkpoint and `Sandbox::run` doesn't, so `DomRuntime::eval` follows each script
with a run of the loop whose options let no task start (`CHECKPOINT`). Other
rejections go in the report (`PageReport::unhandled_rejections`).

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
still ends with `TimeLimit`. The engine can't interrupt a native, so a native's
own loop whose length a script controls either is linear in something a limit
bounds or calls `Document::poll` (a look at the clock every 256 steps).

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
  write and credited on overwrite and drop. Also what script objects keep: the
  names of `getElementsBy*` lists and `Attr`s, and the values of removed `Attr`s,
  until the document drops. These strings are outside the JS heap
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

- **Selectors.** `:has()`, `:nth-child(… of S)` and `:dir()` are refused with
  a `SyntaxError`, as blitz-dom's parser configuration has them. `:enabled`,
  `:disabled`, `:checked`, `:required`, `:optional`, `:lang()`, `:defined` and
  `:read-only` parse and match nothing. `[a=b i]` is case-sensitive, and
  attribute selectors don't look at the namespace. `:scope` in a
  `DocumentFragment` query matches nothing (Selectors 4 makes the fragment's
  top-level elements its children). WPT dom/nodes/selectors.js covers several.
- **Selector limits.** To keep hostile selectors off the stack and out of Rust's
  heap, these valid selectors are a `SyntaxError`: longer than 128 KiB, more
  than 128 combinators on one path through the nested lists, or nesting more
  than 32 brackets, counted with those in strings (`[a="(((…"]`) and comments.
  Telling those apart needs the CSS tokenizer's every rule, and a wrong guess
  would let real nesting reach the parser's recursion, so the count stays
  conservative. `tests/hostile.rs` runs the worst allowed selector on the
  stack headroom alone.
- **`document.cloneNode()`.** §4.4 and WPT Node-cloneNode.html give a new
  `Document`; a runtime holds one document and blitz-dom can't make a second
  document node, so the call ends with `DomError::Unsupported`. That is a gap,
  not conformance (`tests/mutation.rs`, `cloning_a_document_is_a_known_gap`).
  Comes with `DOMImplementation`.
- **Half a surrogate pair.** Text is stored as UTF-8, which can't hold a lone
  surrogate, so a CharacterData offset inside a pair (`deleteData(0, 1)` on an
  emoji) leaves U+FFFD. The DOM keeps the lone code unit.
- **Serialization.** blitz-dom 0.2.4's `outer_html` doesn't escape text
  (`<` stays `<`) and recurses per level, so `Document::to_html` has its own
  iterative serializer (HTML §13.3): HTML, SVG and MathML elements, attribute
  names with their `xml:`/`xmlns:`/`xlink:` prefixes, raw text and void
  elements for HTML elements only, text and comments (`tests/serialize.rs`).
  The tree is parsed and serialized with scripting enabled, unlike blitz-html
  0.2.0: scripts run on it, so `<noscript>` content is raw text (the tree a
  script would see in a browser), and is written back literally. fulgur
  parses with scripting off, so it builds that content as markup and may
  print it: the hand-off (E2-08) has to drop `<noscript>`. E2-02 adds
  `<template>` contents.
