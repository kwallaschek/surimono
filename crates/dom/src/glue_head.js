// The JavaScript half of the DOM bindings: this head, the generated
// generated/bindings.js and glue_tail.js make one function. It is called
// once per context, before any page script runs, with the native functions;
// it returns the function that queues a page's lifecycle, kept by Rust, and
// leaves scripts no way to reach any of them.
//
// A wrapper is an ordinary object carrying a state record { g, h, m } in a
// private class field (#s), stamped on by `Brand`: g is the document's
// generation, h the handle, m the set of interface numbers it brands for (the
// concrete interface and its ancestors), w the wrapper, plus what glue_events.js keeps there. Private fields can't be read, written, listed or
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
  let call = n.call;
  const current = n.current, root = n.currentRoot, stale = n.stale;
  const report = n.report, throwDom = n.throwDom;
  const G = globalThis;
  // The prelude's, before any script can replace them; glue_tail.js puts
  // its own in their place.
  const setTimeoutReal = G.setTimeout, setIntervalReal = G.setInterval;
  const clearTimeoutReal = G.clearTimeout;
  const requestFrameReal = G.requestAnimationFrame;
  const queueMicrotaskReal = G.queueMicrotask;
  const performanceNow = G.performance === undefined ? undefined : G.performance.now;
  const performanceObject = G.performance;
  const now = () => (performanceNow === undefined ? 0 : apply(performanceNow, performanceObject, []));
  const create = Object.create;
  const defineProperty = Object.defineProperty;
  const setPrototypeOf = Object.setPrototypeOf;
  const apply = Reflect.apply;
  const rget = Reflect.get, rset = Reflect.set, rhas = Reflect.has, rdelete = Reflect.deleteProperty;
  const rownKeys = Reflect.ownKeys, rdefine = Reflect.defineProperty;
  const rdescriptor = Reflect.getOwnPropertyDescriptor;
  const ProxyCtor = Proxy, FunctionCtor = Function;
  const indexOf = String.prototype.indexOf, slice = String.prototype.slice;
  const arrayProto = Array.prototype;
  const arrayForEach = arrayProto.forEach, arrayKeys = arrayProto.keys;
  const arrayValues = arrayProto.values, arrayEntries = arrayProto.entries;
  const symbolIterator = Symbol.iterator;
  const toWellFormed = String.prototype.toWellFormed;
  const charCodeAt = String.prototype.charCodeAt, fromCharCode = String.fromCharCode;
  const fround = Math.fround;
  const PromiseCtor = Promise, promiseResolve = Promise.resolve;
  const DOMExceptionCtor = G.DOMException, decodeURI_ = decodeURIComponent;
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

  // Interface objects, prototypes and brand sets by interface number.
  const ctors = create(null), protos = create(null), masks = create(null), names = create(null);
  // Proxy handlers of legacy platform objects, by interface number.
  const exotics = create(null);
  // Bodies of hand-written members (glue_events.js); the window's record.
  const H = create(null);
  // Each event interface's init dictionary converter, by interface number.
  const eventInits = create(null);
  let windowState = null;

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
      s.w = w;
      const handler = exotics[i];
      // Both the target (for the traps) and the proxy (for members) carry
      // the record; private fields don't go through proxy traps.
      if (handler !== undefined) { w = new Brand(new ProxyCtor(w, handler), s); s.w = w; }
      cache[h] = w;
    }
    return w;
  }

  // The record of `this` for a member of interface `id`. An undefined or
  // null `this` is the global object (WebIDL §3.7.6), for
  // `addEventListener(...)`.
  function self(o, id) {
    const s = stateOf(o === undefined || o === null ? G : o);
    if (s === null || s.m[id] !== true) throw new TE('Illegal invocation');
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
    if (s === null || s.m[id] !== true) throw new TE('Argument is not a ' + names[id]);
    if (s.g !== g) stale();
    return s.h;
  }

  // `(Node or DOMString)`: a handle or a string. Rust tells them by type.
  function nodeOrStr(x, g) {
    const s = stateOf(x);
    if (s === null || s.m[I_Node] !== true) return str(x);
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

  // Splits a newline-joined list, or one with a newline after every name
  // (where a name may be empty), without an array's setters: names can't
  // contain a newline. `defineProperty` with a null-prototype descriptor
  // reads nothing a script can have changed.
  function nameList(joined, each) {
    let at = 0;
    while (at < joined.length) {
      let end = apply(indexOf, joined, ['\n', at]);
      if (end < 0) end = joined.length;
      each(apply(slice, joined, [at, end]));
      at = end + 1;
    }
  }
  function nameArray(joined) {
    const out = [];
    let n = 0;
    nameList(joined, (name) => {
      const d = create(null);
      d.value = name; d.writable = true; d.enumerable = true; d.configurable = true;
      defineProperty(out, n++, d);
    });
    return out;
  }

  // WebIDL §3.7.1: a subclass's instance gets the subclass's prototype.
  function adopt(w, target, id) {
    if (target !== ctors[id]) {
      const p = target.prototype;
      if ((typeof p === 'object' && p !== null) || typeof p === 'function') setPrototypeOf(w, p);
    }
    return w;
  }

  // `(boolean or ImportNodeOptions)`.
  function deepFlag(x) {
    // null isn't a boolean, so it is an empty dictionary: deep.
    if (x === null) return true;
    if (isObject(x)) return !x.selfOnly;
    return !!x;
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
    // One set per interface, shared by its wrappers.
    const m = create(null);
    if (parent >= 0) for (const k in masks[parent]) m[k] = true;
    m[id] = true;
    masks[id] = m;
  }
  function konst(id, name, value) {
    const d = { value, enumerable: true };
    defineProperty(ctors[id], name, d);
    defineProperty(protos[id], name, d);
  }
  // WebIDL §3.7.6: a [Global] interface's members are the global's own.
  const holder = (id) => (id === I_Window ? G : protos[id]);
  function attr(id, name, get, set) {
    defineProperty(holder(id), name, { get, set, enumerable: true, configurable: true });
  }
  function op(id, name, f) {
    defineProperty(holder(id), name, { value: f, writable: true, enumerable: true, configurable: true });
  }
  function staticOp(id, name, f) {
    defineProperty(ctors[id], name, { value: f, writable: true, enumerable: true, configurable: true });
  }
  // WebIDL §3.7.6: writing a [Replaceable] attribute makes a data property.
  function replaceable(o, name, v) {
    const d = create(null);
    d.value = v; d.writable = true; d.enumerable = true; d.configurable = true;
    defineProperty(o, name, d);
  }
  // HTML §2.6.1, in terms of the content attribute's text.
  function reflectStr(s, name) {
    const v = call(OP_getAttribute, s.g, s.h, name);
    return v === null ? '' : v;
  }
  function setReflectStr(s, name, v) { call(OP_setAttribute, s.g, s.h, name, str(v)); }
  function reflectBool(s, name) { return call(OP_getAttribute, s.g, s.h, name) !== null; }
  function setReflectBool(s, name, v) {
    if (v) call(OP_setAttribute, s.g, s.h, name, '');
    else call(OP_removeAttribute, s.g, s.h, name);
  }
  function iterable(id) {
    const p = protos[id];
    op(id, 'forEach', arrayForEach);
    op(id, 'keys', arrayKeys);
    op(id, 'values', arrayValues);
    op(id, 'entries', arrayEntries);
    defineProperty(p, symbolIterator, { value: arrayValues, writable: true, configurable: true });
  }
  function stringifier(id, name) {
    op(id, 'toString', function toString() { self(this, id); return this[name]; });
  }

  // The array index a property key spells, or -1.
  function indexKey(k) {
    if (typeof k !== 'string') return -1;
    const n = k >>> 0;
    return `${n}` === k && n !== 4294967295 ? n : -1;
  }

  // WebIDL §3.9 legacy platform objects: indexed and named properties in
  // terms of the operations in `x` (operation numbers). Traps read the
  // record off the target, which scripts never see.
  function exotic(id, x) {
    const ops = create(null);
    for (const k in x) ops[k] = x[k];
    // WebIDL §3.7.8: an interface with an indexed getter gets @@iterator,
    // with or without `iterable<>`'s other members.
    if (ops.item !== undefined) {
      defineProperty(protos[id], symbolIterator, { value: arrayValues, writable: true, configurable: true });
    }
    const length = (s) => call(ops.length, s.g, s.h);
    const itemAt = (s, i) => {
      const r = call(ops.item, s.g, s.h, i);
      return ops.itemWraps ? wrap(s.g, r) : r;
    };
    const named = (s, t, k) => {
      if (typeof k !== 'string' || ops.named === undefined) return undefined;
      if (!ops.overrides && rhas(t, k)) return undefined;
      const r = call(ops.named, s.g, s.h, str(k));
      if (r === null || r === undefined) return undefined;
      return ops.namedWraps ? wrap(s.g, r) ?? undefined : r;
    };
    // The property's value, or undefined if the object doesn't supply it.
    const indexed = (k) => ops.item !== undefined && indexKey(k) >= 0;
    const lookup = (t, k) => {
      const s = stateOf(t);
      if (indexed(k)) { const i = indexKey(k); return i < length(s) ? itemAt(s, i) : undefined; }
      return named(s, t, k);
    };
    // WebIDL §3.9 "named property visibility": own properties hide a name,
    // and so does the prototype chain, without [LegacyOverrideBuiltIns].
    const visible = (t, k) => (ops.overrides ? rdescriptor(t, k) === undefined : !rhas(t, k));
    const h = create(null);
    h.get = function (t, k, r) { const v = lookup(t, k); return v !== undefined ? v : rget(t, k, r); };
    h.has = function (t, k) { return lookup(t, k) !== undefined || rhas(t, k); };
    h.getOwnPropertyDescriptor = function (t, k) {
      const v = lookup(t, k);
      if (v === undefined) return rdescriptor(t, k);
      const d = create(null);
      d.value = v; d.configurable = true;
      d.writable = !indexed(k) && ops.set !== undefined;
      d.enumerable = indexed(k) || !ops.unenumerable;
      return d;
    };
    // Whether a write to `k` goes to the named setter. With `knownOnly`,
    // only names the getter knows do (CSS properties); others are expandos.
    const settable = (t, k) =>
      typeof k === 'string' && ops.set !== undefined && (ops.overrides || !rhas(t, k)) &&
      (!ops.knownOnly || named(stateOf(t), t, k) !== undefined);
    const store = (t, k, v) => {
      const s = stateOf(t);
      // CSSOM's attributes are [LegacyNullToEmptyString].
      call(ops.set, s.g, s.h, str(k), ops.knownOnly && v === null ? '' : str(v));
    };
    h.set = function (t, k, v, r) {
      // WebIDL §3.9.2: only when the object itself is the receiver, not
      // something that inherits from it.
      if (stateOf(r) !== stateOf(t)) return rset(t, k, v, r);
      if (indexed(k)) return false;
      if (settable(t, k)) { store(t, k, v); return true; }
      return rset(t, k, v, r);
    };
    h.defineProperty = function (t, k, d) {
      if (indexed(k)) return false;
      if (settable(t, k)) { store(t, k, d.value); return true; }
      return rdefine(t, k, d);
    };
    h.deleteProperty = function (t, k) {
      const present = lookup(t, k) !== undefined;
      if (!present) return rdelete(t, k);
      // CSSOM's attributes live on a prototype: deleting does nothing.
      if (ops.knownOnly && !indexed(k)) return true;
      if (indexed(k) || ops.del === undefined) return false;
      const s = stateOf(t);
      call(ops.del, s.g, s.h, str(k));
      return true;
    };
    h.ownKeys = function (t) {
      const s = stateOf(t);
      const keys = create(null);
      let n = 0;
      if (ops.item !== undefined) {
        const len = length(s);
        for (let i = 0; i < len; i++) keys[n++] = `${i}`;
      }
      if (ops.named !== undefined) {
        nameList(call(1, s.g, s.h), (name) => {
          if (!indexed(name) && visible(t, name)) keys[n++] = name;
        });
      }
      const own = rownKeys(t);
      const m = own.length;
      for (let i = 0; i < m; i++) keys[n++] = own[i];
      keys.length = n;
      return keys;
    };
    exotics[id] = h;
  }

