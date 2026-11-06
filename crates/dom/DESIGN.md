# surimono-dom design notes

Phase-2 scaffolding. Nothing here is public API: the crate is `publish = false`
and the facade doesn't re-export it. This note records the P1-DOM-02 spike
(id-based handles from rquickjs to blitz-dom 0.2.4) and what E2-02 should keep
or change.

## Handles, not pointers

JavaScript never holds a pointer, an rquickjs class instance or a blitz node id.
A node wrapper is a plain object carrying a record `{ g, h, m }` (generation,
handle, interface set) in a private class field (`#s`, stamped on by a
closure-private class in `src/glue_head.js`); members brand-check with
`#s in o` and the set, one null-prototype object per interface. Private fields can't
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
feature detection sees the truth, with one caveat: every element's interface
exists, members or not, so `typeof HTMLCanvasElement` is `function` though
`getContext` isn't there, and a page that tests the constructor and then calls the
method gets a TypeError where it would have taken its fallback (for COMPAT). The hand-written glue is only infrastructure:
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

## Several documents

A render can hold many documents (`new Document()`, `DOMImplementation`,
`document.cloneNode()`), and blitz-dom has one document node per `BaseDocument`.
Each extra document is an element in the same slab, named `#document`, which
no script sees as an element: `kind` says `Document`. So:

- A node moves between documents by a change of owner: no copy, no new handle,
  and its wrapper stays good. `owners` maps nodes outside the page's document to
  their document, so an ordinary page has an empty map and pays one `is_empty`.
- One set of limits covers every document, because there is one slab, one handle
  table and one byte count. A document bomb ends in `NodeLimit`; nothing about a
  document grows outside them but its `DocInfo`, one per marker.
- A document has a handle from birth, so it lasts as long as the render, like any
  node with a handle.
- Nothing else lives per document. `doc.body` and `getElementById` take `this`;
  whether a node is in an HTML document is `owners` then `DocInfo`.

`Document::enter`, run before each member, sets `making` to `this`'s document. New
nodes join it through `born`, so `createElement`, `innerHTML`, `textContent`,
`append("x")`, `splitText`, `cloneNode` and `importNode` needed no argument.
Parser nodes don't pass through `born` and take a walk after the parse.

Insertion and `adoptNode` run DOM's "adopt", linear in the subtree. `run_page`
collects `<script>`s from the page only, and one moved to another document before
its turn doesn't run (HTML "execute the script element"). `to_html` writes only
the page.

blitz-dom has no doctype, processing instruction or CDATA section either. The
first two are comment nodes to it, the last a text node, with the rest in
`specials`: its layout then skips them, as it does comments. The page's doctype is
one too, so `document.firstChild` is the doctype, as in a browser, and
`node_count` is one higher than the tree of elements suggests. `HTML serialization`
of the document leaves it out, because `to_html` starts at the document element.

**Gaps.** A document other than the page has no browsing context (`defaultView` and
`location` are null, as the standard says), but event handlers don't know that yet:
an `onclick` content attribute in one runs on a dispatched click and
`d.body.onclick` reads back, where HTML §8.1.7.1 gives null, and `d.body.onload` sets
the page's `window.onload`. No wider than a page's own scripts, which could run the
same code; it matters only for conformance. `innerHTML` in an XML document parses and
serializes as HTML on this branch. The template contents
owner document doesn't exist: a template's content stays in the template's document.
`createDocumentType` follows the current "valid doctype name". A `splitText` of a
CDATA section gives a CDATA section's data in a Text node, as the standard does.
`Attr` isn't a whole `Node` (see COMPAT).

## DOMParser

`parseFromString` makes a document like any other in the render: html5ever's tree
builder runs over the same `Sink`, with the new document's marker as its document,
so style text, `style` attributes, templates and limits work as for the page. The
scripting flag is off, as the standard says for a document with no browsing context,
so `<noscript>` content is markup. A refused parse takes back all it made. The
parser goes by chunks and looks at the clock between them. Nothing runs: no script,
no `load`.

## XML

`src/xml.rs` is a hand-written XML 1.0 + Namespaces parser and DOM Parsing's XML
serializer. Both look a name up by hash, undo at the end tag what an element added
instead of copying a scope per element, and hold a namespace once (an interned
`Namespace` in the parser, a borrow of the document in the serializer), so a long
namespace on many nodes costs nothing per node. Serializer output stops at
`max_text_bytes`, because declarations repeated per child can make it far larger
than the tree. A crate would bring code we don't need and doesn't bound what we care
about; this one is 700 lines, uses `Document`'s constructors, so every limit holds,
and is iterative. It reads no external entities and defines none from the internal
subset, which it skips: a reference to any entity but the five predefined ones is a
well-formedness error, so there is no entity bomb and output is no larger than
input. XML 1.0 §5.1 allows that for a non-validating processor; browsers do define
internal entities, a departure for COMPAT. Encoding declarations are checked for
shape and ignored, since the input is already text.

Input that isn't well-formed gives a `parsererror` document, as DOMParser's
algorithm says, whose text is our message. `innerHTML` in an XML document parses
XML in the context's namespaces, and such input is a `SyntaxError`.

`Document::parse_xml` loads a page as XHTML; the document's `html` flag then decides
names everywhere `owners` would, through `page_html`.

## Frames

Scope, chosen for what the selector and namespace tests need and for attack
surface: an `<iframe>` gets a document in the render's slab, and no browsing
context. No script in it runs, nothing navigates, nothing fetches: `src` goes to the
page's resolver, the one external scripts use, and no resolver means no frame.

- `contentDocument` is `about:blank` from the first read while connected, and null
  when not (removal discards it).
- `contentWindow`, and `defaultView` in a frame, are objects inheriting from the
  page's window, with their own `document`. There is no realm: `frames[0].Node ===
  Node`. Tests of realms fail, on purpose.
- The resolver is asked between calls, and the parse happens inside one, one native
  call per frame (`loadFrame`): only there can the time limit and a cancel be seen
  (`Deadline::passed` is false with no call running), and only there does the panic
  guard apply. The source waits in Rust meanwhile; a resolver's answer is the
  caller's to bound.
- `run_page` asks for the parser's frames before any script. They parse in the
  lifecycle's first step and fire `load` before the window's. Frames a script adds
  load when the page has settled, in rounds (at most `MAX_FRAME_ROUNDS`), so a `load`
  comes after every ready timer, at a point that doesn't depend on a resolver's
  speed. They don't delay the window's `load`, unlike in a browser. Each round has
  the time limit, checked between rounds, so a page can take about twice it. A render
  starts at most `MAX_FRAMES` loads; the rest stay blank and fire nothing.
- A frame's type goes by its URL's extension, there being no Content-Type.
- Setting or removing `srcdoc`, and `src` on a frame with no `srcdoc`, in no
  namespace, loads an `<iframe>` again in the next round (HTML §4.8.5), a new document
  under the same `MAX_FRAMES`. The old window keeps the old document, where a browser's
  window object would move on. A reload with nothing to load, such as `about:blank`
  with no resolver answer or `srcdoc` removed with no `src`, keeps the document it
  has and fires `load`.
- A frame's window has its own `setTimeout`, on the page's clock, whose callbacks run
  only while the frame is fully active: its `<iframe>` is connected, up the chain of
  frames to the page. A string handler does nothing and returns 0, since no script
  runs in a frame. Its other timers are the page's.
- `frames` is a list built per read, not the window, and `window[0]` doesn't exist.
- `__surimonoFrameLoads` is a global, non-enumerable and fixed, because Rust has no
  other way into the glue between settles that survives a sandbox rebuild. A script
  calling it fires only what was due.

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
ordinary expandos. `getComputedStyle` is phase 3; see Layout queries.

## Layout queries

Nothing lays out in phase 2, so the brief's policy stands in: `LayoutPolicy`, set
with `DomRuntime::set_layout_policy` (internal; PKG exposes it). `Zeros`, the
default, answers as a browser does for an element with no box, which is an answer
pages already handle: zeros, an empty `DOMRectList`, a null `offsetParent` and
`elementFromPoint`, a read-only computed style with no properties. `Throw` ends
the call with `DomError::LayoutQuery`, fatal so that a `try` can't go on without
the answer. `src/glue_layout.js` holds the members, each asking Rust first through
one operation, so phase 3 replaces one file. They aren't in the generator's table
because `DOMRect` lives in the glue and a native can't return one.

Bound as far as the policy goes: `getBoundingClientRect`, `getClientRects`,
`client*`, `scroll*` (setters ask too, and scroll nothing), `offset*`,
`elementFromPoint`, `getComputedStyle`, and the window's `innerWidth`, `innerHeight`,
`outerWidth`, `outerHeight`, `scrollX`, `scrollY`, `pageXOffset`, `pageYOffset`.
`scroll`, `scrollTo`, `scrollBy` and `scrollIntoView` on elements (the first three on
the window too) move nothing, `elementsFromPoint` hits nothing, and `getBBox` is a zero
`DOMRect`, on every SVG element where SVG 2 has it on graphics elements only. `Throw`
applies to all of them. `pseudoElt` is converted and ignored.
`DOMRectList` has `item` but no indexed properties, since it is always empty.

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
`DomRuntime::cancel_flag` hands the same flag to the caller, for a cancel from
outside: no fault is recorded then, so the call is `Cancelled`. A sticky stop in core, as for a panic, would remove that wait.

**Event handlers** (HTML §8.1.7): `handlers(id, names)` in `bindings.js` comes
from the IDL's `EventHandler` attributes, for each bound interface that has them.
A handler's state is in the target's record, next to its listeners, and
`activate` adds a listener of its own in the place the spec gives it.
Content attributes live in Rust, so `syncHandlers` compares them when an event
reaches the target, a listener is added or the handler is read, which are the
only points a page can observe a listener's place from. Raw text compiles once,
on first use. On a body or frameset element the Window-reflecting set is the
window's. `document.currentScript` is the glue's, set from a script's run until
a separate lifecycle step after its microtasks, which then fires its `load`.

**Event interfaces.** Every interface inheriting from `Event` is generated
whole (`JS_INTERFACES` in the generator): the constructor reads its dictionary
through a generated `D_<name>` converter, which follows WebIDL §3.2.18 (inherited
dictionaries first, members in lexicographic order), and an attribute with a
dictionary member of its name reads the record's `d`, as DOM §2.5 says. What
isn't in a dictionary is hand-written (`H.MouseEvent_pageX`, the legacy
`init*Event`). A value of an interface with no binding can only be null, since
no script can have one.

**Activation** (`glue_activation.js`): `dispatch` finds the activation target as
DOM §2.9 says and runs what `<a>`, `<area>`, `<input>`, `<button>`, `<label>`
and `<summary>` do. A page is printed, not used, so nothing leaves the document:
a form's `submit` and `reset` events fire and stop where it would navigate, and
a link only runs a `javascript:` URL, in a task. `checked` stays an attribute,
which is what gets printed, so a form's reset can't restore it.

**Named access on the window** is WebIDL's named properties object, between
`Window.prototype` and `EventTarget.prototype`: a getter for each `id` in the
parsed tree, made before the first script, at most 4,096. Not a Proxy: QuickJS
reads a global through the prototype chain, and a Proxy there makes an undeclared
name `undefined` instead of a ReferenceError.

**A checkpoint between listeners.** An event fired from a task of the page's life
(`readystatechange`, `DOMContentLoaded`, `load`, a script's `load` and `error`), for
a rejected promise, or by `AbortSignal.timeout` (at the signal and at the signals
that depend on it) has no script under its dispatch, so "clean up after running
script" finds the stack empty after each listener and that is a microtask checkpoint
(`L1,micro,L2`). The glue marks those events (`alone`, set by `fireInTask`) and calls
`n.checkpoint`, which is core's `LoopBridge::checkpoint`. An event from `dispatchEvent`
or from `click()` gets none, trusted or not: a script is on the stack (`L1,L2,micro`,
as in Chrome). A limit in those microtasks comes back from the native typed and ends
the call as `n.limit` does; a listener that overflowed the stack runs no checkpoint.
The checkpoint comes before inner invoke reports the listener's exception and with
`window.event` still set, as WebIDL's order gives; an `error` event reported from such
a listener has one after each of its listeners too. Chrome reports first (`L1, E1, E2,
m, em, L2`, the spec gives `L1, m, E1, em, E2, L2`). The spec wins for DOM behaviour;
the conflict is in `docs/DECISIONS.md`.

**Gaps** (each for docs/COMPAT.md): a
handler's scope has no form owner; MathML elements have no `on*` attributes (no
interface bound), and `<svg><script>` doesn't run; `rejectionhandled` doesn't fire.
`toggle`, `hashchange` and an iframe's `load` come from tasks but get no checkpoint
between listeners, so they give `L1,L2,micro` (Chromium 153 and HTML: `L1,micro,L2`).
ErrorEvent `filename`, `lineno` and `colno` are always empty and 0.
`Event.timeStamp` is the virtual clock, so it is 0 at the start of a page, and
two events at one virtual time have the same one. `HTMLDocument` is `Document`, as
HTML says; browsers keep a separate interface, so `document.constructor.name`
differs from Chrome's. `offsetX`, `offsetY`, `layerX` and `layerY` are the client
coordinates until layout. Interactive form validation doesn't stop a submit.
`TouchEvent`'s `touches` lists are plain arrays, and `Touch` isn't bound, so they
can only be empty. Named access doesn't read `name` and doesn't see what a
script adds. A body element's `on*`
attributes reach the window from the last one written, not from each in turn.
Radio groups key on a `form` attribute's text, not on the element it names.
Setting `details.open` from a script queues no `toggle`; only a click on its summary
does. Two elements with one `id` name the first on the window, not a collection.
A form's reset restores neither `value` nor `checked`. `PointerEvent`'s tilt and
altitude pairs aren't converted into each other. `DeviceMotionEvent`'s members are
the converted dictionaries, not interfaces.

## Location and history

`src/glue_location.js`. `Location` and `History` are hand-written over the
document's URL, which Rust holds and parses (the `url` crate, already a dependency).
`Document::set_url` gives the page its URL; without one it is `about:blank`, and
`location` answers for that.

A printed page goes nowhere, so nothing leaves the document. What would load another
document stops, as a link and a form's submit do, with no error: the page carries on
and prints as it stands. `location.search = ...`, `reload()` and `go(0)` do nothing.
What stays in the document happens: a fragment (a new entry, then `hashchange`), a
`javascript:` URL, `pushState`, `replaceState` and traversal among the entries they
made, with `popstate`. `Document.URL`, `baseURI`, the URLs of `DOMParser`'s documents
and resolved URL attributes all follow.

Session history is in the glue, per document, so it is under the memory limit and
goes with the render. A state is cloned once when pushed: primitives, arrays, plain
objects, `Date`, `RegExp`, `Map`, `Set`. Functions, symbols and platform objects
are a `DataCloneError`, as they should be; `ArrayBuffer`, typed arrays and `Error`
objects clone as plain objects, a gap. A script-set URL is charged to
`max_text_bytes` until nothing holds it, because each `DOMParser` document keeps the
URL it was made under, so a replaced URL can outlive its replacement. One URL is
capped at 2 MiB, for memory: Chrome takes longer ones. Over the cap, `pushState`
gives the `SecurityError` it has for a URL it won't take, `location.hash` does
nothing, and `href`, `assign` and `replace` give a `SyntaxError`. A refusal leaves
session history as it was.

A fragment navigation fires `popstate` at once and `hashchange` in a task; a
navigation to the URL already current is a replace that keeps the state. Until the
page's `load` has fired, a `Location` navigation replaces. A link acts on its own
document, and only for an empty `target`, `_self`, or on the page `_parent` and
`_top`. A frame's fragment fires nothing at the page.

A frame's window has its own `location`, `history` and `DOMParser`. The last is the
one exception to "no realm": a parser's URL is that of the document of the global it
was made in, and nothing else about `frames[0]` differs for it. Its prototype is the
page's, so `frames[0].DOMParser.prototype.parseFromString.call(parser)` works either
way round.

**Gaps.** `[LegacyUnforgeable]` isn't honoured: `Location`'s members are on its
prototype, and a script can replace them; on a frame's window, `location` and
`history` are writable data properties, so `frames[0].location = '#z'` replaces the
object. `ancestorOrigins` isn't bound. No `history.state` survives anything, there
being no reload. `history.length` is per document, where a browser's is joint over
frames. A state is one object for its entry, so `history.state.m = 1` survives a
traversal away and back. The clone keeps no identity for a `Date` or `RegExp`
referenced twice. It clones as plain objects what a browser refuses (`Proxy`,
`Promise`, `WeakMap`, `WeakSet`, `Object(Symbol())`, generators, `arguments`), loses
the value of `new Number(1)` and the like, and takes `RegExp.prototype` for a RegExp.
`<base target>` is ignored.

## MutationObserver

Registrations and record queues are in Rust, which sees the mutations
(`Document::queue_record`); records are objects behind handles, so they count
against `max_handles` and a script that never yields ends in `HandleLimit`. The
callback is in the glue. Once an observer exists the glue's `call` binding
becomes `observedCall`, which queues the notify microtask when a record waits; a
page without observers pays nothing.

Strings in Rust are charged to `max_text_bytes`: a registration's filter plus
`REGISTRATION_COST` (credited on replace and `disconnect`), and a record's old
value and attribute name, for the document's life, because handles are never
released. An old value is copied only if an observer asked, and `check_consistency`
counts all of this as `observer_bytes`.

An observer's number belongs to one document. The glue keeps the observers of
the current generation only, so another document's records never reach an old
observer, and `observe`, `disconnect` and `takeRecords` on one call `stale()`.

"Replace" and "replace all" (DOM §4.2.3) queue one record: `replaceChild`,
`replaceWith`, `outerHTML`, `replaceChildren`, `textContent`, `innerHTML`.
Adopting a node from elsewhere still queues its own, as does a fragment for the
children it loses.

Gap: no transient registrations for removed subtrees.

## Ranges

`src/range.rs`. A range is two boundary points in Rust, because Rust sees the
mutations; the script holds a handle to `Object::Range`, a number in
`Document::ranges`. The hooks sit in the primitives (`place_quietly`,
`detach_quietly`, `replace_child_list`, and a fragment's children leaving in
`insert_checked`) and in "replace data", "split" and `normalize`, so no path that
changes the tree can miss them. `replace_child_list` removes many children in one
pass over the ranges, with the result of removing them one at a time.

Every node a boundary point names has a handle, so its slab slot isn't reused. A
removal moves a boundary to the parent, which gets a handle then; out of handles,
the boundary stays on a live node, a fault is recorded and the call fails.

- **Bounds.** A range can't be known to be unreachable (wrappers have no weak
  references), so it stays live until the document drops, and a mutation may have
  to look at every range: `DomLimits::max_ranges` (65,536) ends in the typed
  `RangeLimit`. A page with no ranges pays one `is_empty`. `range_points` counts
  boundary points per node, so an insertion or data change elsewhere pays one
  lookup, and a removal walks the removed subtree only up to about twice the
  number of ranges. Past that it walks every range's ancestors under `poll`,
  which is the `TimeLimit`; a walk stopped there leaves boundaries out of date in
  a script that is already ending, so nothing here relies on them being right:
  methods clamp, and a range out of one tree answers with an exception.
- **Depth.** "Clone the contents" and "extract" recurse per partially contained
  level in the standard. Here they walk two chains, the left one's contents
  leaving from the bottom, the right one's from the top, which is the order the
  recursion has for mutation records.
- **What goes with a render.** All of it is in `Document`.
- **`normalize`** goes step by step once a range exists, like with an observer:
  quadratic on a wide list, and each removal pays for it in its poll.

`StaticRange` is `Object::StaticRange`; its constructor reads the dictionary in the
glue. `getClientRects` and `getBoundingClientRect` go through the layout policy, in
`glue_layout.js`; a range's are zeros until phase 3.
`createContextualFragment` is HTML §8.5, with the wrapper stripping of XML documents.

**Gaps.** There is no `Selection`: no vendored test needs one but `moveBefore`'s,
which fail on `moveBefore` first. An `Attr` can't be a boundary point (`Attr` isn't
a whole `Node`): `InvalidNodeTypeError`, where the standard allows offset 0. Scripts
in a contextual fragment don't run when appended, since no script a script inserts
does yet. A range's offsets inside a surrogate pair have the same lossy edge as
CharacterData's.

## Templates and reflected attributes

A `<template>`'s content is a fragment outside the tree, in `Document::templates`.
Everything that walks what a node owns goes through `push_owned`, so limits,
bytes and drops see it. `input.value` and `checked` write the attribute rather
than a dirty state: what a script fills in has to reach the printed page. URL
attributes parse without a base, so a relative one reads back as written.

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
with a run of the loop whose options let no task start (`CHECKPOINT`).

Other rejections follow HTML's "notify about rejected promises". The checkpoint
queues a task behind what is already queued, so the next `<script>` or a timer already
due can still handle the promise. The task calls the hook the glue registered
(`n.setRejectionHook`), which fires `unhandledrejection` at the window, cancelable,
with a checkpoint after each listener; the promises left go in the report
(`PageReport::unhandled_rejections`) unless a listener cancels the event, in the order
they rejected. No limit error gets that far, so no listener sees or cancels one. A
page's lifecycle steps were queued first, so the event comes after `load` for a
rejection in a page script; Chrome's parser can yield earlier. If the event loop stops
with the task still queued (`TaskLimit`), its promises are counted with no event.

One residual: core reads a rejection's reason to report it, and a getter on the
reason runs then. If that getter overflows the stack, the overflow is an
exception in the read and the rejection counts as an ordinary one, not as a limit.
The script gets nothing out of it: the getter's stack is gone, and the next
overflow on the way to a native ends the call as usual.

## Sandbox and limits

Every native's body runs under `Deadline::guard` through `guarded`, so a panic
ends the call with `HostPanic` however the script handles the exception, and
rquickjs doesn't leave it stored for a later call (#102). A panic may have
stopped a change to the tree halfway, so the document goes with the script's
state. `every_native_is_guarded` reads this file and fails on a native without it.

Every script runs through `surimono_core::Sandbox::run`, so the memory, time
and stack limits, determinism and the no-I/O rule apply unchanged; crates/dom
needed nothing new from core. The glue is compiled once per process and loaded as bytecode into each context
(`load_glue`), through the sandbox's own `compile` and `eval_bytecode`; measured
at opt-level 1, a page's install went from 23 ms to about 15 ms, about what it was
with half the interfaces, and it adds 1.4 MB of JS heap (0.9 MB before e2-13c).
`DomRuntime::eval` is two sandbox calls, the script
and its checkpoint, with their own deadlines, so a hostile script can use up to
twice `Limits.time` there; `run_page` is one call for everything after install. Bindings are installed inside the first call on
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

**Parsing outside a script.** `Document::parse` runs before any script and no
sandbox limit covers it. html5ever compares each attribute with the tag's others,
so one tag costs quadratic time in its length, in calls of its own: 60,000 attributes
in 420 KB took 13.8 s. `Document::parse_stoppable` takes a stop check for that, and
the caller's cancel and deadline belong in it. The feed asks between chunks, of
256 bytes while no node appears, so the wait is one small feed. `max_attributes`
(4,096, the printing pass's cap) is a typed `AttributeLimit`, on the parser, on
scripts and in XML; html5ever only shows a tag at its end, so the cap alone doesn't
bound a hostile tag's time. `<html>` and `<body>` merge repeated tags' attributes,
and those count together.

`parse_xml_stoppable` does the same for XML, asked as tokens go by. XML's cost is
linear (names are hash lookups), so its stop is for a large input, not a shape.

`Document::to_html_bounded` is `to_html` with a byte cap and a stop check, looked at
per node and every 64 KiB of a string.

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

- **Selectors.** `:has()`, `:nth-child(… of S)`, `:dir()`, `::first-line` and
  `::first-letter` are refused with a `SyntaxError`, as blitz-dom's parser
  configuration has them. `:indeterminate`, `:default`, `:placeholder-shown`,
  `:valid` and `:invalid` parse and match nothing. `:enabled`, `:disabled`,
  `:checked`, `:required`, `:optional`, `:read-only`, `:read-write`, `:lang()`,
  `:defined` and `:target` are answered in `select.rs`; `checked` and `selected` are
  the attributes, as everywhere here, and `:target` is the URL's fragment, in the
  document queried. `:scope` in a
  `DocumentFragment` query matches nothing (Selectors 4 makes the fragment's
  top-level elements its children). WPT dom/nodes/selectors.js covers several.
- **Selector limits.** To keep hostile selectors off the stack and out of Rust's
  heap, these valid selectors are a `SyntaxError`: longer than 128 KiB, more
  than 64 combinators on one path through the nested lists, or nesting more
  than 8 blocks (brackets and functions). Depth is counted on cssparser's
  tokens, which the selector parser reads too, so what sits in a comment, a
  string or an escape doesn't count either way; a byte scan can't do this, since
  a `)` in a comment would hide a real level. `tests/hostile.rs` runs the worst
  allowed selector from the deepest frame a script can reach, on a thread of
  `stack_bytes`, the headroom and 64 KiB for what comes before the sandbox's
  stack check (some of which stays free, so it can't see use just past the
  headroom). Frames aren't known at compile time, so the caps are
  set from measurements with room to spare: that selector takes just over half
  of the 256 KiB headroom in the slowest x86_64 build and under a third in
  release (numbers at `MAX_SELECTOR_NESTING`). `Guarded` also looks at the stack
  left at each element it reaches (`surimono_core::stack_available`), and under
  64 KiB stops the match and ends the call with `SandboxError::StackOverflow`,
  sticky like a refused allocation, so a target whose frames outgrow the measurement
  gets that error and not a crash. Selector parsing runs before `Guarded` and
  can't be stopped, so its stack still rests on the caps. On a platform where
  core can't tell the stack left, so does matching.
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
