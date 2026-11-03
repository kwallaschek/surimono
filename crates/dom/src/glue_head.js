// The JavaScript half of the DOM bindings: this head, the generated
// generated/bindings.js and glue_tail.js make one function. It is called
// once per context, before any page script runs, with the native functions;
// it returns nothing and leaves no way to reach them.
//
// A wrapper is an ordinary object carrying a state record { g, h, m } in a
// private class field (#s), stamped on by `Brand`: g is the document's
// generation, h the handle, m the interface bits (the concrete interface and
// its ancestors). Private fields can't be read, written, listed or
// intercepted through any prototype, Proxy or reflection, so scripts can
// neither read nor forge a handle; `#s in o` is the brand check. Every
// member passes the pair to Rust, which resolves it through the document on
// each call. Wrappers hold no pointers and keep no Rust node alive.
//
// Nothing here looks up a built-in at call time: the intrinsics used after
// install are captured below, records and caches are null-prototype objects,
// and natives take and return only primitives (an array built by Rust would
// be filled through index setters a script can install), so a script that
// replaces Object.create, Map or WeakMap methods, Function.prototype.call or
// Array index setters can't observe or change what the bindings do.
(function (n) {
  'use strict';
  const call = n.call, current = n.current, root = n.currentRoot, stale = n.stale;
  const create = Object.create;
  const defineProperty = Object.defineProperty;
  const setPrototypeOf = Object.setPrototypeOf;
  const apply = Reflect.apply;
  const toWellFormed = String.prototype.toWellFormed;
  const TE = TypeError;
  const symbolTag = Symbol.toStringTag;

  // `new Brand(o, s)` returns `o` itself with the private field added (a
  // base constructor that returns an object makes it the derived class's
  // `this`).
  class Stamp { constructor(o) { return o; } }
  class Brand extends Stamp {
    #s;
    constructor(o, s) { super(o); this.#s = s; }
    static of(o) { return typeof o === 'object' && o !== null && #s in o ? o.#s : null; }
  }
  const stateOf = Brand.of;

  // Interface objects, prototypes and brand masks by interface number.
  const ctors = create(null), protos = create(null), masks = create(null), names = create(null);

  // Same node, same wrapper (DOM identity), within one document generation.
  let cacheGeneration = -1;
  let cache = create(null);

  function wrap(g, h) {
    if (h === 0) return null;
    if (g !== cacheGeneration) { cache = create(null); cacheGeneration = g; }
    let w = cache[h];
    if (w === undefined) {
      const i = call(0, g, h);
      const s = create(null);
      s.g = g; s.h = h; s.m = masks[i];
      w = new Brand(create(protos[i]), s);
      cache[h] = w;
    }
    return w;
  }

  // The state of `this` for a member of interface `id`.
  function self(o, id) {
    const s = stateOf(o);
    if (s === null || (s.m & (1 << id)) === 0) throw new TE('Illegal invocation');
    return s;
  }
  function need(argc, min, what) {
    if (argc < min) throw new TE(what + ': ' + min + ' argument(s) required, ' + argc + ' present');
  }

  // WebIDL conversions. A template literal is exactly ToString (a Symbol
  // throws a TypeError). Lone surrogates can't reach the UTF-8 side, so they
  // become U+FFFD here, not at the boundary.
  function str(x) { return apply(toWellFormed, `${x}`, []); }
  function obj(x, id, g) {
    const s = stateOf(x);
    if (s === null || (s.m & (1 << id)) === 0) throw new TE('Argument is not a ' + names[id]);
    if (s.g !== g) stale();
    return s.h;
  }

  // `(Node or DOMString)`: a handle or a string. Rust tells them by type.
  function nodeOrStr(x, g) {
    const s = stateOf(x);
    if (s === null || (s.m & nodeBit) === 0) return str(x);
    if (s.g !== g) stale();
    return s.h;
  }
  // A call with a variadic argument list. Reflect.apply reads a
  // null-prototype list, so no Array method or setter is involved.
  function vcall(op, s, args, conv) {
    const n = args.length;
    const l = create(null);
    l.length = n + 3;
    l[0] = op; l[1] = s.g; l[2] = s.h;
    for (let i = 0; i < n; i++) l[i + 3] = conv(args[i], s.g);
    return apply(call, undefined, l);
  }

  function iface(id, name, parent, ctor) {
    const f = ctor === null ? function () { throw new TE('Illegal constructor'); } : ctor;
    const proto = f.prototype;
    defineProperty(f, 'name', { value: name, configurable: true });
    defineProperty(f, 'prototype', { writable: false });
    defineProperty(proto, symbolTag, { value: name, configurable: true });
    if (parent >= 0) {
      setPrototypeOf(f, ctors[parent]);
      setPrototypeOf(proto, protos[parent]);
    }
    ctors[id] = f; protos[id] = proto; names[id] = name;
    masks[id] = (parent >= 0 ? masks[parent] : 0) | (1 << id);
  }
  function konst(id, name, value) {
    const d = { value, enumerable: true };
    defineProperty(ctors[id], name, d);
    defineProperty(protos[id], name, d);
  }
  function attr(id, name, get, set) {
    defineProperty(protos[id], name, { get, set, enumerable: true, configurable: true });
  }
  function op(id, name, f) {
    defineProperty(protos[id], name, { value: f, writable: true, enumerable: true, configurable: true });
  }

