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
fails when they are stale. Only listed members exist in JavaScript, and the plain
reflections (below), so a page's feature detection sees the truth, with one caveat: every element's interface
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

Layout needs none of the three back in this document: phase 3 lays out a copy
(see "Layout queries (phase 3)"), which gets the sheets and the style attributes
as stylo's own objects, parsed here, and never runs a hook.

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
`noscript` text in a template's contents serializes escaped, as the spec and Chrome
have it, though the contents are in the page's document here: their own owner
document would have no browsing context.
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

Phase 3's design and its prototype (`LayoutPolicy::Real`, hidden) are in the next
section; this one is what ships until checkpoint 3.

Nothing lays out in phase 2, so the brief's policy stands in: `LayoutPolicy`, set
with `DomRuntime::set_layout_policy` (internal; PKG exposes it). `Zeros`, the
default, answers as a browser does for an element with no box, which is an answer
pages already handle: zeros, an empty `DOMRectList`, a null `offsetParent` and
`elementFromPoint`, a read-only computed style with no properties. `Throw` ends
the call with `DomError::LayoutQuery`, fatal so that a `try` can't go on without
the answer. `src/glue_layout.js` holds the members, each asking Rust first through
one operation (`layoutQuery`, which also says whether real numbers are to be had,
so the default path pays nothing for the prototype), so phase 3 replaces one file. They aren't in the generator's table
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

## Layout queries (phase 3)

The E3-00 spike: how scripts get real numbers from Blitz's layout, what it costs,
and what blitz-dom 0.2.4 can and can't answer. Source references are to
blitz-dom 0.2.4 unless they name another crate. A prototype is in
`src/layout.rs` and `src/layout_blitz.rs` behind `LayoutPolicy::Real` (hidden,
not reachable from the facade, the CLI or the gem): `getBoundingClientRect`,
`offsetWidth` and `offsetHeight`, tested in `tests/layout_real.rs`. Nothing here
is public API before checkpoint 3.

### The seam: bindings on one side, an engine on the other

fulgur's maintainer is building a second layout engine (Raikiri, an opt-in
backend in fulgur 0.41; Blitz stays the default and there is no date for that
to change). So everything a layout query needs goes through one boundary, and
only one file knows which engine answers:

```
glue_layout.js ── layoutQuery / layoutNumber ──▶ Document (src/layout.rs)
                                                   │ stale? (version, layout_epoch)
                                                   │ Engine::lay_out(&Document, &LayoutSetup)
                                                   ▼
                                           Box<dyn Laid>  ◀── src/layout_blitz.rs (Blitz, the mirror)
                                                          ◀── src/layout_raikiri.rs (Raikiri, feature `raikiri`)
```

- **`LayoutSetup`** is what comes in from outside the page: the viewport and
  the engine with its fonts, `LayoutEngine::Blitz(FontContext)` or, with the
  `raikiri` feature, `LayoutEngine::Raikiri(RaikiriSetup)` (E3-R1: font bytes
  and family names with system fonts off, and optionally the page's size and
  margins). Phase 3 targets Raikiri (DECISIONS 2026-11-14); how its engine
  reads the page and answers is in `RAIKIRI.md` section 9.
- **`Engine::lay_out(page, setup)`** turns the page as it is now into a laid-out
  page, or refuses it (`Ok(None)`, answered as no box). It reads the tree, the
  sheets and the style attributes through `Document`'s own accessors
  (`children`, `kind`, `element_name`, attributes, `char_data`, `style_text`,
  `edited_sheet_text`, `style_attr::parse`) and looks at the page's deadline
  where its work can be cut.
- **`Laid`** is what queries are answered from, in the page's node ids, never
  the engine's: `border_box(node)`, and `offset_position(node)` (offsetLeft and
  offsetTop, Raikiri only). E3-01 and E3-02 add their questions
  here and nowhere else: `client_rects(node)`, `box_edges(node)` (padding,
  border, scrollbar, content size), `offset_parent` inputs,
  `computed_value(node, pseudo, property) -> Option<String>`,
  `matches_media(query)`, `hit(x, y)`. Answers are numbers and strings, so no
  engine's types reach the bindings.
- **Staleness** (`Document::ensure_layout`) and the policy are on the bindings'
  side. An engine that can update in place gets a second, optional entry
  (`Laid::update(page, changes) -> bool`, false for "build me again") when
  E3-01 has a change list to give it.

What is not neutral yet: the tree an engine reads is stored in a blitz-dom
slab (see "Risks"). The fonts are each engine's own: a Parley 0.6
`FontContext` for Blitz, font bytes for Raikiri, which shapes with shodo.

### A mirror, not the page's own document

Layout runs on a second `BaseDocument`, the *mirror*, built from the page's
tree when a script first asks and thrown away when the page changes. The
page's own document stays the tree store it is. Laying out in place was the
plan in "Staying out of blitz-dom's document behaviour"; the source says no:

- **Blitz makes nodes while it lays out, in the document's slab.**
  `collect_complex_layout_children` (`layout/construct.rs`) calls `create_node`
  for every anonymous block, and `flush_pseudo_elements` for `::before` and
  `::after` (with a text child). In place they would count against `max_nodes`, take slab ids our handle table
  indexes by, and need telling apart on every walk.
- **Nothing is kept between two `resolve`s in the build that prints.** Without
  the `incremental` feature `resolve_layout_children` (`document.rs`) collects
  the layout tree anew every time (`NON_INCREMENTAL` in `lib.rs`), and fulgur
  0.40.0 depends on `blitz-dom = "0.2"` with default features, which don't
  include it. So keeping one document laid out saves no work, and a fresh
  copy always starts from a known state.
- **Style sheets are ordered by node id.** `add_stylesheet_for_node` keeps
  `nodes_to_stylesheet` in a `BTreeMap` by node id and inserts before the next
  id (its own TODO says ids get reused). The page's ids are in creation order,
  so a `<style>` a script inserts above an older one would cascade in the wrong
  order. A mirror is built in document order, so its ids are too.
- **The hooks that are off must stay off.** Layout in place needs the
  `IS_IN_DOCUMENT` flags back, and with them everything "Staying out of
  blitz-dom's document behaviour" keeps off (loads, form-control children,
  the mutator's own parse of `<style>` text), for every later mutation. The
  mirror is built with `create_node` and plain links
  (`Node::parent`, `Node::children` are public fields), so no hook runs, and its
  sheets are made here with `Stylesheet::from_str(.., None, None, ..,
  AllowImportRules::No)` and given to `add_stylesheet_for_node`, not through
  `make_stylesheet`, which always passes a loader and `AllowImportRules::Yes`.
- **A fault during layout leaves the page's tree whole.** It still ends the
  call with a typed error, as any native's does.

What the mirror costs is one copy of the tree per layout (the text that can
render, and attributes: short values whole, of a long one its first 2 KiB,
`class` and `id` always whole, `style` as its parsed block) and a second set of nodes while it is alive; both are in "Cost" below.
The same builder is what a live hand-off to the printer needs (a prepared Blitz
document, `docs/upstream/`), so it isn't thrown-away work.

### Running style and layout

`BaseDocument::resolve(now)` (`document.rs`) is the whole of it:
`resolve_stylist` (stylo's sequential traversal: `style::driver::traverse_dom`
with no thread pool), `resolve_layout_children` (box construction),
`resolve_deferred_tasks` (Parley shaping of every inline root,
`build_inline_layout_into`), `flush_styles_to_layout` (stylo to Taffy styles,
`stylo_taffy::to_taffy_style`) and `resolve_layout` (`taffy::compute_root_layout`
against `stylist.device().au_viewport_size()`, then `taffy::round_layout`).
Each node ends with `unrounded_layout` and `final_layout` (public fields,
`taffy::Layout`: location relative to the layout parent's border box, border-box
size, border, padding, content and scrollbar sizes) and `layout_parent`.

It needs:

- **A root element.** Without one `resolve` prints `No DOM - not resolving` on
  stdout, which is where the CLI writes the page, so the mirror checks
  `try_root_element` first.
- **A viewport**: `DocumentConfig::viewport`, a `blitz_traits::shell::Viewport`
  of whole pixels (`window_size: (u32, u32)`) and a scale; see "Page geometry".
- **Fonts**: `DocumentConfig::font_ctx`, a `parley::FontContext` the document
  moves into an `Arc<Mutex<_>>`. `BlitzFontMetricsProvider` (`font_metrics.rs`)
  reads it for `ex` and `ch`, and `resolve_deferred_tasks` shapes with it. It
  must be built with `CollectionOptions { system_fonts: false, .. }` from the
  bytes the printer gets, exactly as fulgur 0.40.0's `parse_inner`
  (`blitz_adapter.rs`) does: `register_fonts(blob, None)` per file, in order.
  `FontContext::default()` is never an option: with Parley's `system` feature
  on anywhere in the build (blitz-dom's defaults turn it on, and fulgur uses the
  defaults) it loads the machine's fonts. The prototype takes the context in
  `LayoutEngine::Blitz` and clones it per mirror (a `Collection` clone shares the
  font blobs). **Hand-over from surimono-gfx (E3-01):** a
  `FontSet::font_context()` that returns this context from its own collection,
  with its generic-family mapping applied through
  `Collection::set_generic_families`, so that `sans-serif` measures as
  `TextMeasurer` measures it. Blitz maps stylo's generics to Parley's
  (`stylo_to_parley::generic_font_family`), and a generic with no mapping finds
  no font: the text then has no glyphs and no width.
- **The UA sheet**: `BaseDocument::new` parses `assets/default.css` (21 KiB)
  for every document. It is the fixed cost of an empty layout, below.
- **The time**: `resolve(0.0)`. Animations and transitions take it from this
  argument, never from a clock, so nothing moves.

**Deterministic?** In one process: `the_same_page_gives_the_same_numbers_twice`
lays a page with blocks, flex, a table and wrapped text out in two runtimes and
again after a change, and compares every number. Across processes the inputs
are the tree, the sheets, the font bytes and the viewport; the traversal is
sequential, the hash maps in `BaseDocument` (`nodes_to_id`, `changed_nodes`,
`ua_stylesheets`) are only looked up on this path, and fontique's family order
was the one place hash order showed (`FontSet::add_font_bytes` sorts it, which
is one more reason the context comes from surimono-gfx). Two runs of the test
binary print the same numbers for that page (compared by hand, 11 Nov); a test that compares across
processes belongs to the fixture suite once the facade can reach layout (E3-06).

**Threads.** The style engine keeps a little state per thread for as long as
the thread lives (DECISIONS 2026-11-11, #169). Layout runs on the thread that
owns the document, which in the facade is the long-lived `surimono-page`
thread (`crates/surimono/src/page.rs`); the tests run every layout on one
long-lived thread too.

### Cost

Release build, this machine (2 cores), `tests/layout_real.rs` `layout_cost`,
a report page of flex rows (label and figure, five nodes a row, one style
attribute a row), fixture fonts:

| Page | First layout | A query after it | Layout again after one attribute changes |
|---|---|---|---|
| 20 rows, 107 nodes | 1.6 ms | 0.04 ms | 1.4 ms |
| 2,000 rows, 10,007 nodes | 60 to 81 ms | 0.05 ms | 54 to 75 ms |
| 9,500 rows, 47,507 nodes | 377 ms | 0.06 ms | 367 to 378 ms |
| 19,000 rows, 95,007 nodes (measured before the text bound below, which this page is over) | 801 ms | 0.07 ms | 824 to 853 ms |

About 8 µs a node, of which an almost empty page pays 1.1 ms whatever its size
(a new `BaseDocument` and its UA sheet). Other builds were running on the
machine, so read these to one digit.

These numbers are for this page and are not a bound: a page within every
bound under "Limits" can make one layout take far longer than its size
suggests (review of #179), and nothing bounds one layout's time yet. Nothing
is kept from one layout to the next: changing one attribute costs what the
first layout did. Sheets come on top: `sheet_parse_cost` gives 58 ms for 1 MiB of rules
(6,525 of them, a child combinator and a `:not()` each, five declarations),
about 56 ns a byte.

**Memory.** Resident memory grew by 1.2 MiB (107 nodes), 56 MiB (10,007),
150 MiB (47,507) and 297 MiB (95,007) over the measurement's three layouts, one mirror alive at the
end: 3 to 6 KiB a node with the allocator's slack, several times the page's own
tree. That is the number that matters against the memory target, and the
reason for `max_layout_nodes` below and for dropping the mirror when the
scripts settle, before `to_html`.

### Laziness and invalidation

Layout runs inside the first query that needs it (`Document::ensure_layout`),
never at load and never for a page that doesn't ask. Under `Zeros` and `Throw`
nothing changes and the default path pays nothing: the glue's `layoutQuery` now
answers whether real numbers exist and only then asks for them.

**The first rule: one stale flag for the document.** A layout is stamped with
`(version, layout_epoch)`. `version` already counts every change a live list can
see (tree changes in `place_quietly`, `detach_quietly`, `replace_child_list`,
`remove_children_quietly`, adoption, and every attribute write and removal, so
`class`, `id`, `style`, `hidden`, `width`). `layout_epoch` is new and counts what
`version` doesn't: text (`set_text_data`, `set_side_data`, so a `<style>`'s text
too) and CSSOM rule edits (`sheet_insert`, `sheet_delete`). A query with a
different stamp drops the mirror and builds another. Correct by construction,
and coarse in two ways that E3-01 should fix first because frameworks do both:

1. Changes to nodes outside the page's tree (a fragment being built, a detached
   subtree) make the layout stale though it can't have changed. Counting only
   changes under the page's root is an ancestor walk per mutation, or free where
   the funnel already knows the parent is detached.
2. Attributes no selector or presentational hint reads (`data-*`, `aria-*`
   without a rule) also count. Stylo's invalidation knows; ours doesn't yet.

**What Blitz offers beyond that.** Three levels, in order of cost to adopt:

- *Keep the mirror and replay changes through `DocumentMutator`.*
  `set_attribute` and `clear_attribute` call `snapshot_node`, which fills the
  `SnapshotMap` stylo's invalidation reads, so `resolve_stylist` restyles only
  what a change can reach. Box construction and layout still run in full
  (`NON_INCREMENTAL`), so this saves the copy and most of the style pass, and
  a replayed mirror is still built afresh every so often to start from a
  known state. How much of a layout that is, this spike didn't measure
  (blitz-dom's `log_phase_times` feature prints the split, on stdout); E3-01
  measures before choosing.
- *The `incremental` feature*: damage propagation (`layout/damage.rs`,
  `propagate_damage_flags`), construction only where damage says, Taffy caches
  kept. Off in fulgur's build, so turning it on is a different layout from the
  one that prints unless fulgur does too, and feature unification would turn it
  on for the printer in any build that has both. Not before fulgur.
- *Style without layout*: `resolve_stylist` alone is public. `getComputedStyle`
  of a property that doesn't depend on boxes, and the SVG text queries of
  E3-03, need the cascade and not the boxes, so the mirror should style first
  and lay out only when a box is asked for.

### The style attribute

DECISIONS 2026-11-05: a style attribute never reaches stylo through blitz-dom;
the tree has the text and no element has a `style_attribute` block. The flush is
part of building the mirror, per element, and never
`Node::flush_style_attribute`, which is one `parse_style_attribute` over the
whole attribute that nothing can stop:

1. `style_attr::parse(text, quirks, deadline)`, the parse a `style` read already
   does: declarations one at a time, each validated by stylo, the deadline
   looked at in between.
2. Each kept declaration is parsed by stylo on its own
   (`parse_style_attribute("name: value")`) and its declarations pushed into one
   `PropertyDeclarationBlock`, which goes into `ElementData::style_attribute` (a
   public field) under the mirror's lock. The attribute itself stays in the
   mirror's attribute list, so `[style]` selectors match.

**Bound.** Unchanged caps, all enforced where the attribute is written
(`style_attr::fits`): 4,096 declarations, 65,536 tokens, 8 MiB, 32 brackets,
2^26 push steps, 4,096 grid cells; values over 64 KiB are skipped, as they skip
validation (in practice `data:` images, which move no box). The longest stretch
without a look at the deadline is one declaration of at most 64 KiB, in either
pass, the unit the 2026-11-04 decision already accepts. An attribute over the
caps can't be in the tree; if one were, the page is not laid out.

**Cost.** Every layout parses every style attribute again, twice over
(validation, then the block); the page in "Cost" has one a row, and their
share wasn't split out. E3-01 should keep the parsed block
by node, dropped when that attribute is written, and clone it into each mirror
(`PropertyDeclarationBlock` is `Clone`; only the lock wrapper belongs to one
document).

### Style sheets

Per `<style>` element, in document order: its edited rules
(`edited_sheet_text`) or its text (`style_text`), parsed by stylo with no
loader and `@import` off, so nothing is fetched. `@font-face` rules parse and load nothing (blitz-dom fetches them only
in `CssHandler`); fonts are the caller's.

A sheet's parse can't be stopped once started (DECISIONS 2026-11-06) and
`<style>` text is bounded only by `max_text_bytes` (256 MiB), so every sheet
is screened before the style engine sees any of them, and a page over a bound
is not laid out (it answers as with no box):

- `MAX_SHEET_BYTES`, 1 MiB of sheet text in all (about 60 ms of parsing).
- Its shape, in `src/sheet_screen.rs`, one pass over cssparser's tokens that
  nests no deeper than its own bound and looks at the deadline every 4,096
  tokens: 32 levels of nesting (`{}`, `()`, `[]` and functions, so nested
  rules and selector functions alike), 4,096 declarations in a block, 2^26
  steps over the page's blocks (each charged its declarations squared, an
  `all` as 512 more), 4,096 words in the strings of a grid or custom-property
  declaration and 65,536 in the page, 64 combinators in a selector, and 64
  links in a chain of custom properties naming each other through `var()`,
  the style attributes' declarations included.

These are the printer screen's numbers (`crates/surimono/src/print.rs`), which
has the same job for the page that is printed, and the two must agree: a
page the printer takes should be one layout takes. This crate can't depend on
the facade, so the pass is written twice; one shared screen is a follow-up
for the orchestrator. The combinator count here is on tokens, not on parsed
selectors, so it can refuse a prelude the printer screen would pass (an
at-rule's with more than 64 spaces in it).

Not there yet, for E3-01 and E3-02: the `media` attribute of `<style>` (blitz-dom
gives every sheet `MediaList::empty()`), `disabled`, and **linked sheets**.
`link.sheet` is null and linked sheets "arrive after settle" today, so a page
whose CSS is in a `<link>` would be measured without it. With real layout the
facade has to give linked sheets to the document before the page's scripts
run, as a browser blocks scripts on them (request to PKG below).

### Which queries, and from what

All positions are sums of `unrounded_layout.location` up the `layout_parent`
chain to the root element (iteratively; `Node::absolute_position` recurses and
uses the pixel-snapped layout), in CSS pixels at scale 1. Nothing scrolls, so
viewport and document coordinates are the same.

| Query | From | Notes |
|---|---|---|
| `getBoundingClientRect` | border box: position as above, `unrounded_layout.size` | prototype. Zeros with no box (CSSOM View §6.1) |
| `getClientRects` | one rectangle for a block-level or atomic box; for an inline element the glyph runs whose `brush.id` is the element, per line, from the inline root's `inline_layout_data.layout` (`lines()`, `items()`, `PositionedLayoutItem::GlyphRun`: `offset`, `advance`, the line's metrics) | `TextBrush::id` is the styled node's id (`node/element.rs`), so runs map back to spans |
| `offsetWidth`, `offsetHeight` | border-box size, rounded | prototype |
| `offsetTop`, `offsetLeft`, `offsetParent` | position relative to the nearest positioned ancestor's padding edge; the ancestor from computed `position` up the tree (CSSOM View §7), not from `layout_parent`, which can be an anonymous block | |
| `clientWidth`, `clientHeight`, `clientTop`, `clientLeft` | `size` less `border` less `scrollbar_size`; `border.top`, `border.left`. The root element answers the viewport (§6) | |
| `scrollWidth`, `scrollHeight` | `max(client size, content_size)`; `Layout::scroll_width()` is the overflow only | `scrollTop` and `scrollLeft` stay 0: nothing scrolls in print |
| `innerWidth`, `innerHeight`, `document.documentElement.clientWidth` | the setup's viewport | `outerWidth` the same; `scrollX`, `scrollY` 0 |
| `getComputedStyle` | `Node::primary_styles()` gives stylo's `ComputedValues`; `computed_value_to_string(PropertyDeclarationId)` (stylo `properties.mako.rs`) serialises any longhand as a resolved value, which is Servo's `getComputedStyle` serialisation: colours as `rgb()`/`rgba()`, lengths in `px`, keywords lower case. Custom properties from `custom_properties()`. Shorthands are put together from longhands by `PropertyDeclarationBlock`'s serialisation or by hand for the few pages read (`font`, `margin`, `padding`, `border-*`) | Properties: stylo's Servo set of longhands. Not there: `fill`, `stroke` and the other SVG presentation properties, `break-inside` (see "Inline style"), so E3-03's text needs them from attributes and the cascade it does itself. Used values (CSSOM §9: `width`, `height`, margins, paddings, `top`/`left`... when the element has a box) come from the box, not from the style. `::before`/`::after`: `stylo_element_data.styles.pseudos` |
| `matchMedia` | stylo: `MediaList::parse` and `evaluate(doc.stylist_device(), quirks)` | The device is `MediaType::screen()` (`make_device`, `document.rs`), and fulgur prints with it, so `matchMedia('print')` is false and `(max-width: ..)` answers for the page's content width. That is what a script sees in Chrome-based tools too (they run scripts under `screen` and print afterwards). A `print` device needs `set_stylist_device` with a `Device::new` of our own, whose `FontMetricsProvider` blitz-dom doesn't export (`BlitzFontMetricsProvider` is `pub(crate)`): possible, but only worth it if the printer does the same. No `change` events |
| `document.fonts.ready` | a resolved promise; `document.fonts.check()` from the font set | fonts are all there before the first script; `@font-face` loads nothing |
| `elementFromPoint` | `BaseDocument::hit(x, y)`, mapped back through the mirror | not needed for print; last |
| `Range` rectangles | as `getClientRects` for inline content | after elements |

**For SVG (E3-03, Graphics).** blitz-dom lays out the `<svg>` element as a
replaced box (`compute_child_layout`: `width`/`height` attributes or the
style's size; with the `svg` feature it serialises the subtree and parses it
with usvg, with the feature off, as here, the children are just not boxes).
Nothing inside it has a box. So crates/dom hands over, per query, and
surimono-gfx computes: (1) the outer `<svg>`'s border box and content box from
the mirror, for `getBoundingClientRect` of inner elements and for percentage
lengths against the viewport; (2) for any element in the subtree, stylo's
computed `font-family`, `font-size`, `font-weight`, `font-style`,
`font-stretch`, `letter-spacing`, `word-spacing`, `text-anchor`-like properties
as attributes (stylo has no SVG ones), `display` and `visibility`, so
`getComputedTextLength` and a `<text>` `getBBox` measure with the cascade's
font, and `display: none` gives no box; (3) the subtree itself (names,
attributes, text) by node id, which gfx's own SVG geometry walks for
`getBBox` of shapes, groups and paths, with `transform` attributes. A style-only
resolve is enough for (2), which matters: D3's label loops call `getBBox` once
per label after appending it.

### Page geometry

The pre-pass lays out against the width the printer will: fulgur 0.40.0
(`engine.rs`) passes `config.content_width().as_pt().in_px()` to `parse_inner`,
which builds `Viewport::new(width as u32, height_px, 1.0, ColorScheme::Light)`.
So the viewport is the page width less the left and right margins, in CSS
pixels at 96 dpi, **truncated to a whole pixel** (A4 with 20 mm margins:
642.52, so 642), by the full page height, also truncated (`engine.rs` passes
`config.page_height()`, not the content height: A4 is 1122), scale 1, light
scheme, `screen` media. `LayoutSetup { width, height, engine }` with Blitz's
fonts is exactly that. The numbers come
from the caller's printer settings, which surimono doesn't know today: the
facade needs a page option (size, margins, orientation; default A4 and
fulgur's default margins) that it converts as above and that callers set to
what they give fulgur. That option is phase-3 public API and waits for
checkpoint 3 (request to PKG below). A caller who prints with another engine
gives that engine's content box.

Chrome-based tools differ here on purpose: they run scripts against the
browser window (800 by 600 in Puppeteer) and lay out again for paper, so a
chart sized by script keeps its window size. Measuring at the paper's width is
what makes a script-sized chart fill the printed column.

### Limits

Layout runs on the host, where the engine can't interrupt. What bounds it:

- **Before it starts**: every native looks at the deadline first, and the copy
  looks again per node (`Document::poll`), per style-attribute declaration
  (in both passes over it), every 4,096 tokens of a sheet's screen and per
  sheet. A script that thrashes (write, read, write, read) is stopped by its
  own time limit between layouts.
- **One layout** (style, construction, shaping, Taffy) is one call that can't
  be stopped once started, so everything that goes into it is bounded before
  it starts, and a page over a bound is not laid out and answers as with no
  box (`a_page_over_the_layout_bounds_is_not_laid_out`):
  - nodes and depth, by the page's own `max_nodes` (100,000) and `max_depth`
    (512);
  - text, `MAX_TEXT_BYTES`: 256 KiB in all, about 80 printed pages. Only text
    that can render counts and is copied: not a script's source, a title,
    `noscript` or `template` content, a sheet's text, or the children of
    `img` and `canvas` (`unrendered_text_and_long_attributes_dont_stop_a_layout`).
    Text under `display: none` still counts, since that isn't known before
    style;
  - attributes, `MAX_ATTRIBUTE_BYTES`: 8 MiB copied in all, a value over
    2 KiB counting and copied as its first 2 KiB (`class` and `id` whole);
  - sheets, `MAX_SHEET_BYTES` and the shape bounds under "Style sheets";
  - style attributes, by `style_attr::fits` when they were written.

  The page's `max_text_bytes` (256 MiB) is no bound for a layout, which is
  why the engine has its own. These bounds, and the sheet screen's, bound what
  a layout is given; they don't bound how long it runs. The screen bounds
  parsing, not matching or layout, and one layout of a page within every
  bound can run far past the script's deadline (review of #179). **A layout
  is not yet bounded in time**, so `LayoutPolicy::Real` stays `doc(hidden)`
  and unreachable from the facade, the CLI and the gem until E3-01e bounds it
  (DECISIONS 2026-11-13; "Open points for E3-01"). E3-01 should also make the
  bounds `DomLimits` and report a refusal instead of answering silently.
- **Proposed for E3-01**, the cheapest thing that keeps the time limit honest
  without new machinery: a layout is refused (typed, reported, answers as with
  no box) when the page has more than `max_layout_nodes` nodes, default set
  from the measured rate so that one layout stays under the default time
  limit; and a count of layouts per render in the report, with a cap (default
  1,000) that turns further queries into answers from the last layout, so a
  page in a measure loop degrades instead of timing out. A node count bounds
  the copy's size, not one layout's time, so this comes after E3-01e's time
  bound, not instead of it. No stop check inside
  Blitz: `resolve` has no hook for one, and a `[patch]` wouldn't reach users of
  the published crates.
- **Stack.** What nests in a layout is the tree, bounded by `max_depth`, and
  the sheets, bounded at 32 levels by the screen (which itself nests no
  deeper). `the_deepest_tree_the_default_limits_allow_lays_out` lays out a
  tree 508 deep on a thread of the page thread's size (8 MiB over the
  script's stack); measured once on a debug build, the same tree fitted a
  768 KiB thread.
- **Memory.** The mirror is outside the JavaScript heap limit and outside
  `max_text_bytes`; it is bounded by the node limit and the two bounds above,
  and there is one at a time. At `max_nodes` that was about 300 MiB resident
  ("Cost"), which is too much to leave to the node limit alone.

### What blitz-dom 0.2.4 can't answer

Found reading the source and probing the prototype; each needs a COMPAT row
once the query is real.

- **Generic font families need a mapping.** With system fonts off and no
  generic mapped, text in `sans-serif`, or in the UA default `serif` of a page
  that names no family, has no glyphs: a paragraph is 0 high and an inline-block
  0 by 0. It prints blank the same way (DECISIONS 2026-10-31, #3), so measuring
  agrees with printing, and both want the mapping from surimono-gfx above.
- **Replaced elements aren't sized by their attributes.** `<canvas width=50
  height=20 style="display:block">` is 800 by 0 in an 800px viewport:
  `replaced_measure_function` (`layout/replaced.rs`) lets the stretched width
  win over `attr_size`, and the height comes from a 0/0 aspect ratio. An inline
  canvas is no box at all (`find_inline_layout_embedded_boxes` knows `img`,
  `svg`, `input`, `textarea` and `button`), and in this build, without
  blitz-dom's `svg` feature, an inline `<svg width=120 height=40>` is 0 by 0.
  Presentational `width`/`height` reach stylo only for tables and `hr`
  (`synthesize_presentational_hints_for_legacy_attributes`, `stylo.rs`). This
  is the first thing E3-01 has to fix, since a chart is a canvas or an svg: the
  mirror adds one UA-origin sheet (`add_user_agent_stylesheet` is public) with
  a rule per distinct size it copied,
  `canvas[width="50"][height="20"] { width: 50px; height: 20px }`, plus
  `canvas { display: inline-block; width: 300px; height: 150px }` and the same
  for `svg`, `img`, `video` and `iframe`. UA origin loses to every author rule,
  as a presentational hint should, and the text is made here from numbers.
  `a_canvas_has_the_size_of_its_attributes` is the ignored test that waits for
  it. The mirror already leaves out the children of `img` and `canvas`, which
  HTML never renders.
- **The root element's margin is ignored**: with `html { margin: 7px }` a box
  at the top of the body is at 0,0, where CSS puts it at 7,7
  (`taffy::compute_root_layout` places the root at the origin). `body`'s margin
  is honoured.

- **Inline elements have no box.** A `<span>` in running text is not a layout
  child (`find_inline_layout_embedded_boxes` takes only inline-blocks and
  replaced elements), so the prototype answers zeros for it. Its rectangles can
  be had from the Parley layout (table above); until then
  `span.getBoundingClientRect()` and `span.offsetWidth` are wrong where
  `inline-block` and block elements are right. The most common measuring idiom
  (an absolutely positioned or `inline-block` element, then `offsetWidth`) has a
  box.
- **Shrink-to-fit text widths are whole pixels.** `compute_inline_layout`
  (`layout/inline.rs`) takes `calculate_content_widths()` and applies `.ceil()`,
  so an inline-block around text Chrome measures at 67.57 is 68 wide. The text
  inside is laid out exactly as surimono-gfx measures it
  (`crates/gfx/tests/fulgur_layout.rs`). Never narrower than Chrome, at most a
  pixel wider.
- **`line-height: normal` is `font-size * 1.2`** in `compute_child_layout`
  (`layout/mod.rs`) for form controls and `resolve_line_height`
  (`layout/construct.rs`), where Chrome takes the font's ascent, descent and
  line gap (Noto Sans: 1.36), and so does
  `stylo_to_parley::style` for every line of text
  (`LineHeight::Normal => FontSizeRelative(1.2)`). An inline-block of 12px Noto
  Sans is 14.4 high here, where Chrome's font box is 17 (13 + 4,
  `chrome_measure_text.json`), and it sits 0.4px above its line. Text set in the
  fixture fonts without a `line-height` is about 12% shorter than Chrome's, in
  print too, so pages should set one, and this goes to Q-GFX-01 as a print gap.
- **Quirks mode is never on**: `Stylist::new(device, QuirksMode::NoQuirks)` and
  `TDocument::quirks_mode` (`stylo.rs`) are fixed, and the stylist is private.
  A page with no doctype is laid out in standards mode (and printed so).
- **Media is always `screen`**, as above.
- **`position: absolute` inside an inline formatting context** is placed by a
  stand-in (`layout/inline.rs`, "TODO: Implement absolute positioning").
- **Display types**: Taffy lays out block, flex, grid and (as grid) tables;
  `display: inline` content through Parley. No floats (`float` is parsed and
  ignored: two `float: left` blocks stack), no multi-column in Blitz itself (fulgur has its own), `::before`
  and `::after` only with a single string for `content`, no `::marker` box
  styling, no shadow trees.
- **Form controls** get sizes from constants (`input`: 300 px wide,
  `textarea`: `cols * font-size * 0.6`).
- **`<style>` text and character references**: `process_style_element` decodes
  HTML entities in the sheet's text (`html_escape::decode_html_entities`), which
  a browser doesn't do. The mirror doesn't use it; the printer's own parse of
  our output does, so `a::before { content: "&amp;" }` prints differently from
  Chrome. For Q-GFX-01.
- **Things it prints on stdout**: `No DOM - not resolving` (`resolve`, `hit`),
  `ERROR: Tried to lay out text node individually` with a `dbg!`
  (`compute_child_layout`), SVG parse failures with the whole source
  (`collect_layout_children`, `svg` feature only). The first is avoided; the
  second hasn't shown in any test page, and would corrupt the CLI's stdout if
  it did.

### Open points for E3-01

Known and left, from the review of the prototype (#179):

- **E3-01e, first: a time bound on one layout**, style matching and layout
  and not only parsing, the precondition for any public layout API
  (`LayoutPolicy::Real` stays internal until then). The review's two
  directions: a budgeted dry run of the author rules' selectors on the copy
  through `select.rs`'s guarded matcher (`Guarded`, which already charges a
  clock for its work and stops at a deadline), refusing the layout when the
  budget runs out; or a structural bound computed before the layout from the
  selectors and the copy's shape. Either way no ordinary sheet may be refused:
  the review laid out the six real-world pages and the KaTeX, highlight.js,
  Bootstrap 3, Material for MkDocs, Read the Docs, Font Awesome, Tailwind 4
  and pdf.js sheets, each in 1 to 11 ms, and every one of them must still be
  laid out.
- A `<style>` inside a canvas's fallback content or under an `img` isn't
  applied in the copy, which leaves out the children of both, though a
  browser applies it.
- `set_layout_setup` after a layout keeps the old layout: the setup belongs in
  the stamp, or setting it clears the layout. `disabled`, `media` and `type`
  on a `<style>` need an attribute or the epoch behind them when they become
  real; today a `<style type="text/plain">` is applied.
- The mirror's nodes are made without the form-control data the printer's own
  parse creates, so `:checked` never matches although `checked` is in the
  tree, and a `display: block` text input is as wide as its container where
  the printer's has the 300 px constant.
- `[style*="…"]` and other value selectors on `style` don't match in the copy
  (the attribute is there, empty), and a value selector on an attribute over
  2 KiB sees its first 2 KiB.
- Tests to come with the next queries: an inline-block that isn't at the
  line's start, and `offsetWidth` at a fractional position (Chrome snaps by
  position; `layout.rs` rounds the size).
- One screen for style text, shared by this crate and the facade.
- `parley` pinned once, in the workspace.

### Risks and unknowns

| Risk | How to settle it |
|---|---|
| The printer's layout engine stops being Blitz (Raikiri becomes fulgur's default, or a caller prints with `--engine raikiri` today) | See "If the printer's engine changes" below. The seam means a second `Engine`; nothing else is settled until someone reads Raikiri's API for boxes by node, which this spike didn't |
| Numbers differ from Chrome's by more than pages tolerate (line heights, whole-pixel text boxes, no floats) | Probe pages in `wpt/probes` run in the pinned Chromium and in the runner with real layout on: fixed boxes, flex, a table, wrapped text, the measuring idioms of Chart.js and D3. Asked of the Conformance researcher; the runner needs the hidden switch |
| Measured and printed layouts differ because fulgur changes the document before it lays out (its DOM passes, pagination, `set_viewport_size_px`) | One fixture printed by fulgur with boxes drawn at the rectangles a script read; E3-06 |
| Measure loops on large pages are slow under the whole-document flag | The two refinements under "Laziness" first; then the replayed mirror; measured on the real-world suite's Chart.js and D3 pages (E3-06) before choosing |
| A fault inside the engine on some tree or style | It ends the call with a typed error; the fuzz test (`tests/fuzz.rs`) gets a mode that lays out each generated page, so one is found here and not by users |
| Deep trees and the stack on a caller's thread | Measured above for the page thread; `Document`'s direct users (tests, the runner) must lay out on a thread of that size |
| Linked style sheets missing when scripts measure | PKG: load them before scripts when layout is real |
| `getComputedStyle` serialisation differs from Chrome's in places pages compare strings (`rgb(0, 0, 0)` matches; shorthand forms and `calc()` may not) | The css/cssom-view and cssom WPT slices of E3-06, plus probes for the properties Chart.js reads (`width`, `height`, `padding-*`, `border-*-width`, `box-sizing`, `display`, `position`, `max-width`, `max-height`) |
| Memory with the mirror alive on the soak | `tests/soak.rs` gains a layout per render in E3-01; the mirror drops with the document |

### If the printer's engine changes

Measuring agrees with printing only when both use the same layout. If fulgur
prints with Raikiri, what has to change, from least to most:

1. **A second `Engine`** in a file of its own: the walk `layout_blitz.rs` does,
   into Raikiri's DOM, and `Laid` answered from its boxes. The glue, the host
   operations, staleness, the policy and the tests' expectations (which are the
   spec's and Chrome's) stay as they are. Whether Raikiri gives boxes by node
   to an embedder at all is the first thing to find out.
2. **Choosing the engine** becomes part of the setup, because a caller can pick
   the printer's backend per call: the facade's page option (see "Page
   geometry") grows an engine field, default Blitz while fulgur's is.
3. **`LayoutSetup::fonts`**: one Parley version in the build or font bytes
   instead of a context (above).
4. **Computed values.** `getComputedStyle` is stylo's serialisation here. An
   engine with its own style system either serialises resolved values itself
   behind `Laid::computed_value`, or computed style keeps coming from stylo
   while boxes come from the other engine, which can disagree with what is
   printed (a property one cascade supports and the other drops).
5. **Style-attribute validation** (`style_attr.rs`) asks stylo because stylo is
   "the parser the printer uses" (DECISIONS 2026-11-04). Under another parser
   that reason is gone: what stylo refuses the printer might use. The caps stay.
6. **The tree store.** Our DOM lives in a blitz-dom slab: `BaseDocument` is the
   node storage (`document.rs`, `tree.rs`), html5ever builds into it through
   blitz-html's sink (`parse.rs`), and selectors match through blitz-dom's
   `selectors::Element` for `Node` (`select.rs`'s `Guarded`). None of that is
   layout, and none of it has to move for a second engine: blitz-dom would stay
   as the tree, at the price of building stylo, Taffy and Parley for it and of
   the MPL exception. Moving the tree to a slab of our own is a large, contained
   change to those four files and the `selectors::Element` implementation, and
   only worth it if Blitz itself goes unmaintained.
7. **Pins and drafts.** DECISIONS 2026-10-31 pins Blitz to what fulgur prints
   with; the pin's reason would become Raikiri's Parley and Taffy. The live
   hand-off draft in `docs/upstream/` (a prepared Blitz document) would have no
   taker; serialized HTML is the hand-off any engine takes.

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
An `svg` element fires `load` as SVG 2 §15.2 says, once the document's resources have
loaded: all of them in one step just before the window's `load`, in the order of their
end tags. Chrome differs: a nested `svg` fires as it closes, before `DOMContentLoaded`,
and its order among outermost ones isn't tree order. The spec wins.

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
parsed tree, made before the first script, at most 4,096. Writing follows OrdinarySet: the
value lands on the receiver if it takes it. A refusal is silent, because a setter can't
answer `false`: `Reflect.set` says `true`, and a strict-mode write to a frozen receiver
doesn't throw. Not a Proxy: QuickJS
reads a global through the prototype chain, and a Proxy there makes an undeclared
name `undefined` instead of a ReferenceError.

**A checkpoint between listeners.** An event fired from a task of the page's life
(`readystatechange`, `DOMContentLoaded`, `load`, a script's `load` and `error`, an
`iframe`'s and an `svg`'s `load`), `toggle`, `hashchange`, a traversal's `popstate`, for
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
the conflict is in `docs/DECISIONS.md`. A timer that throws follows the same order
(`T, m, E1, em, E2`); a string handler is a classic script, reported before
its cleanup (`T, E1, E2, m, em`), as in Chrome. `requestAnimationFrame` and
`queueMicrotask` callbacks don't yet.

**Gaps** (each for docs/COMPAT.md): a
handler's scope has no form owner; MathML elements have no `on*` attributes (no
interface bound), and `<svg><script>` doesn't run; `rejectionhandled` doesn't fire.
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
goes with the render. A state is cloned once when pushed, by `cloneValue`, which
`structuredClone` shares (see "What the window has"). A script-set URL is charged to
`max_text_bytes` until nothing holds it, because each `DOMParser` document keeps the
URL it was made under, so a replaced URL can outlive its replacement. One URL is
capped at 2 MiB, for memory: Chrome takes longer ones. Over the cap, `pushState`
gives the `SecurityError` it has for a URL it won't take, `location.hash` does
nothing, and `href`, `assign` and `replace` give a `SyntaxError`. A refusal leaves
session history as it was.

A fragment navigation fires `popstate` at once and `hashchange` in a task; a
navigation to the URL already current is a replace (§7.4.2), which fires the `popstate`
and no `hashchange`. Its entry has no state, as "navigate to a fragment" says: the
classic history API state is never carried over. Chrome keeps `history.state` there;
the spec wins. Until the
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
traversal away and back. `<base target>` is ignored. A `location.hash` in an inline script gets its `hashchange`
after the window's `load`, because the page's life was queued first; Chrome fires it
before.

## What the window has

`src/glue_platform.js` and `src/web.rs`: what isn't about a node. Interfaces come
from the IDL through `JS_INTERFACES`; the state stays in JavaScript values, and Rust
takes and returns strings. Nothing here needs `unsafe`, so bytes cross as a string of
U+0000 to U+00FF, built in chunks through a null-prototype list: slower than a
borrowed slice, and rquickjs's slice is `unsafe`. `TextEncoder.encode` copies into an
`ArrayBuffer` with `new_copy`: `ArrayBuffer::new` would keep the bytes outside the
memory limit.

- **URL.** A `URL` is its serialization in a record. Each getter and setter parses
  it again in Rust through the `url` crate's `quirks` module, which is the URL
  Standard's API. So what the crate gets wrong, we do (below), and there is one
  parser. Input and output are capped at the 2 MiB of `MAX_URL_BYTES`, a `TypeError`
  for a constructor or a setter. Parsing is linear, Punycode included:
  `hostile_hosts_cost_little` runs six shapes at the cap.
- **URLSearchParams.** A list of pairs in the glue. `sort` is a merge sort, so no
  `Array.prototype` method runs; iterators are live, as WebIDL's pair iterators.
  `searchParams` is made on first read, which nothing can observe.
- **TextDecoder.** All of `encoding_rs`, which stylo already links. A decoder is
  stateful and can't be serialized, so one mid-stream stays in `Document::decoders`,
  each charged 4 KiB against `max_text_bytes`, because a script can keep any number
  open; the end of a stream frees it, and the rest goes with the document.
- **structuredClone.** `cloneValue` in `glue_location.js`. `valueKind` tells a Proxy,
  an Error and a Promise apart, since no script-visible member does. A plain object
  takes a fast path; the rest tries each built-in's brand. `transfer` clones, then
  detaches, so a failed clone leaves the buffers. Recursion ends at the stack limit.
- **navigator.** Constants: nothing may vary by machine, clock or version. `platform`
  is empty, `onLine` and `cookieEnabled` are false, since there is no network and no
  cookie store, `hardwareConcurrency` is 1. The user agent is
  `Mozilla/5.0 (compatible; surimono)`.
- **customElements.** A registry and nothing else: `define`, `get`, `getName` and
  `whenDefined` work, nothing is constructed or upgraded, no reaction runs, and
  `:defined` doesn't change. So `customElements.define` no longer stops a script, and
  a page that fills its elements from `connectedCallback` prints them empty.
  Reactions need a hook on every path that adds a node.

**Gaps** (for COMPAT). `Intl` isn't in the engine. `Blob`, `File` and
`URL.createObjectURL` aren't bound, nor are `TextDecoderStream` and streams. From the
`url` crate: a space before `?` or `#` in an opaque path stays a space, where the URL
Standard escapes the last one as `%20`, and removing the query of such a URL strips
its trailing spaces, where the standard keeps them with the last escaped; IDNA is
older than the tests (`CheckHyphens`, `xn--` labels that don't decode). On Windows
drive letters (`file:///w|/m`, a relative `/` against `file://h/C:/a/b`) the crate gives
what the URL Standard says now, and WPT's failures there are its older expectations. `structuredClone` doesn't clone `DOMRect` and the like,
refuses a `SharedArrayBuffer` where the standard shares it, and clones as plain objects
generators and a `Map` given `Object.prototype`. A fatal error in the middle of a
stream drops a byte pushed back before it (`textdecoder-mistakes`, one subtest).
Only the page's window has `atob` and the rest as functions that work: `frames[0].atob`
needs a `this` of the page, like every window operation.

## Tables, forms and links

`src/html.rs` has the lists, as more `ListKind`s, so they are live the way
`getElementsBy*` are, recomputed once per change of the tree. `src/glue_html.js` has the
members that only move nodes, through the DOM's own members captured at install, so a
script that deletes `Node.prototype.insertBefore` changes nothing and every limit
holds on the way. `HTMLOptionsCollection` and `HTMLFormControlsCollection` borrow
`HTMLCollection`'s proxy handler; the first adds the index setter.

- **`selected` is the attribute**, as `checked` is for an input: what a script chooses
  has to reach the printed page. Rust's `selected_options` runs the standard's
  "selectedness setting algorithm" over the attributes on each read: a single select
  has its last `selected` option, or its first enabled one at display size 1. A
  script's `selectedIndex = -1` is a flag in the glue, since no attribute says it, so it
  reads back; `selectedOptions` and the printer still see the first option. And
  `defaultSelected` and `selected` are one thing.
- **`textarea.value`** is its text, which is what gets printed, so `defaultValue` is the
  same.
- **`innerText`** can't know layout. Out of the tree it is `textContent`, as the
  standard says. In the tree it is "rendered text collection" with each element's
  display from the rendering section's defaults, the `hidden` attribute and an inline
  `display: none`; a stylesheet isn't seen until phase 3, and neither are CSS
  `white-space`, `text-transform` or `visibility`. Rows break against the last row of
  the nearest table, across sections, a closed `details` has only its summary, and
  a `select` gives its options on lines of their own, as Chromium 153 does. The setter
  is the standard's.
- **Focus** is a state in the document's record. Being rendered is being connected.
  `focus()` and `blur()` fire `blur`, `focusout`, `focus`, `focusin` synchronously,
  `activeElement` falls back to the body, and an element that leaves the tree loses
  focus without an event. `:focus` doesn't match, there is no sequential navigation
  and no `autofocus`.
- **`document.cookie`** is empty and a write does nothing: no store exists, and a
  render has no one to give cookies to.

**Gaps** (for COMPAT). `form[0]`, `form.name` and `select[0]` don't exist; `namedItem` on
`form.elements` gives the first element where the standard gives a `RadioNodeList` for
several. `document.links` and the rest are `HTMLCollection`s with what the standard's
name rules give, but `document.foo` for a form or image name isn't bound. `checkValidity`
and its family aren't bound. `relList` isn't. `new Image()` loads nothing, so `onload`
never fires. A `select` in another select's option list isn't looked for, as the
standard's newest parsing allows.

## Traversal

`NodeIterator` and `TreeWalker` are in `src/glue_traversal.js`, because a filter is a
script's function and each step can change the tree. A walker has no state outside
its record. A `NodeIterator` must keep working as nodes are removed (§6.1 "pre-removing
steps"), which only Rust sees, so its pointer is in `src/traversal.rs` and
`iterators_removing` sits beside `ranges_removing` on the three paths nodes leave by,
with the result of removing `gone` one at a time. An iterator's pointer can't be known
to be unreachable, so it stays until the document drops; each is charged 4 KiB against
`max_text_bytes`, which caps them at 65,536, and each removal walks up from each
iterator's reference, under `poll`. A list leaving at once costs one map of its
children's neighbours, so `n` children under `m` iterators stay `n + m`. A page with no
iterator pays one `is_empty`. `Attr` isn't a whole `Node`, so `createTreeWalker(attr)` and
`createNodeIterator(attr)` are `TypeError`s, where Chrome takes them.

## Stylesheets

CSS-in-JS writes with `insertRule` into a `<style>` whose text stays empty, so a rule
that stayed in the CSSOM would never be printed. `src/sheet.rs` divides text into
top-level rules on cssparser's tokens, the way the printer's parser does, without
stylo: a stylo parse can't be stopped, and nothing here needs a rule's meaning. Blocks
are skipped by the tokenizer, iteratively, and the cost is linear in the text.
`glue_sheet.js` has what scripts see: a sheet is read when first asked for and is a
list of records in the heap, and a new text makes a new sheet, as in a browser. An
edit also goes to Rust, which keeps the rules of that element, charged against
`max_text_bytes`. `to_html` writes those in place of the element's text, until the text
next changes; `innerHTML` and `textContent` still give the text, as a browser's do.
A generation per style element tells that, and not a compare of the text, because a
browser's new sheet comes with any change, even back to the same text. It counts from
the first read of `sheet`, on the paths nodes join and leave by and in
`set_side_data`, and costs a page with no sheet one `is_empty`.

CSS Syntax ends what is open at the end of its input without an error, so a rule's text
is closed when it is written: `closed` ends a comment, string, `url(` or bracket left
open. Otherwise `insertRule('b{/*')` would swallow every rule after it in the output,
where a browser, which doesn't print rules from text, loses nothing.

So what the printer gets from an edited sheet went through the facade's screen like any
`<style>`, and nothing about `insertRule` is a path around it.

**Gaps** (for COMPAT). No declaration is parsed: `cssText` is the rule's own text with
spacing normalized around the block (`b{top:0}` gives `b { top:0; }` where Chrome gives
`b { top: 0px; }`, and an at-rule keeps its text), `rule.style` isn't bound, and a rule
Chrome would refuse for its selector or declarations stays. So `insertRule('!!! {}')`
and `selectorText = '!!!'` succeed where Chrome gives a `SyntaxError` and changes
nothing; a selector can't be judged without taking the printer's view of what is
known. Grouping rules have no `cssRules`; a `@media` rule is one
`CSSRule` of type 4 and `CSSMediaRule` isn't bound. `link.sheet` is null and
`document.styleSheets` has only `<style>`s, because linked sheets arrive after settle.
`media`, `disabled`, `replace`, `replaceSync`, `adoptedStyleSheets` and `new
CSSStyleSheet()` aren't bound. An `svg` `<style>` has no `sheet`. Removing and
re-inserting a style element keeps its sheet and its edits, where Chrome makes a new
one from the text.

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
than a dirty state: what a script fills in has to reach the printed page.

**A write that only made a property would lose what the script meant to print**
(`el.hidden = true` printed a visible element until P2-DOM-09). So:

- **Plain reflections come with their interface, listed or not.** Every writable
  attribute of an element interface that the IDL marks `[Reflect]`, `[ReflectURL]`,
  `[ReflectNonNegative]`, `[ReflectPositive]` or `[ReflectPositiveWithFallback]` is
  generated with HTML §2.6.1's getter and setter for its type (`DOMString`,
  `DOMString?`, `USVString` as a URL, `boolean`, `long`, `unsigned long`, `double`),
  with `[ReflectDefault]` and `[ReflectRange]`; the helpers are `reflect*` in
  `glue_head.js`. The IDL leaves "limited to only known values" and the like to
  prose, so a marked attribute needs nothing more. A URL is parsed against the
  document's base URL, or read back as written if that fails.
- **Prose reflections are written by hand**, in the glue (`popover`, `outerText`,
  img, input and canvas `width`/`height`, progress and meter values, output
  `value`/`defaultValue`, title and script `text`) or in `host.rs` (`dir`,
  `document.dir`, the document's legacy colours, `slot`, `base.href`), each with its
  section's steps; `tests/reflection.rs` checks them.
- **The audit.** `the_audit` in `tests/reflection.rs` writes every writable (or
  `[PutForwards]`) attribute of the element interfaces, `SVGElement` and `Document`
  on an element in the page, fails if a plain reflection is wrong, and prints what
  each other write did: `writes the tree`, `keeps the value` (state the page can't
  print), `no effect`, `unbound` (an expando: nothing exists) or what it threw.
  Event handler attributes are left out.

An img's `width` is a layout query only when the element is connected (§4.8.17:
"being rendered"); under `LayoutPolicy::Zeros` it has no box, so the attribute
answers, and no image is ever available, so with no attribute it is 0.

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

`parse_xml_stoppable` does the same for XML, asked as tokens go by and every 64 KiB
inside a long one: text, a run of references, white space. XML's cost is linear (names
are hash lookups), so its stop is for a large input, not a shape.

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
  document queried, from the first script on: Chrome sets the target at the end of
  parsing, so an inline script finds nothing there. `:scope` in a
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
- **Attributes a write doesn't reach (P2-DOM-09's audit).** Unbound, so a write is
  an expando: `style.disabled` and `document.adoptedStyleSheets` (a sheet a script
  turns off still prints), `input.valueAsNumber` and `valueAsDate`, the
  `DOMTokenList`s other than `classList` (`relList`, `iframe.sandbox`, `link.sizes`,
  `blocking`, `output.htmlFor`), the enumerated ones limited to known values that
  don't print (`crossOrigin`, `referrerPolicy`, `loading`, `decoding`,
  `fetchPriority`, `as`, `formEnctype`, `formMethod`, `popoverTargetAction`,
  `inputMode`, `enterKeyHint`, `shadowRootMode`, `shadowRootSlotAssignment`,
  `track.kind`), `translate`, `draggable`, `spellcheck`, `autocorrect`,
  `writingSuggestions`, `autocapitalize`, `contentEditable`, `nonce`,
  `autocomplete`, `formAction`, `command`, the `…Element` reflections,
  `dialog.returnValue` and `closedBy`, `script.async`, `marquee.loop`,
  `document.domain` and `designMode`. State the printed page can't carry:
  `input.indeterminate`, and a `select` with no option selected
  (`selectedIndex = -1`, a `value` no option has), which prints its first option.
  `scrollTop` and `scrollLeft` are layout (phase 3). And the reverse: `className`
  on an SVG element is `Element`'s string, where SVG 2 gives an `SVGAnimatedString`
  that a write doesn't change.
