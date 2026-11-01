// The JavaScript half of the DOM bindings. Called once per context, before
// any page script runs, with the native functions; returns nothing and
// leaves no way to reach them.
//
// A wrapper is an ordinary object carrying its [generation, handle] pair in
// private class fields (#g, #h), stamped on by `Brand`. Private fields can't
// be read, written, listed or intercepted through any prototype, Proxy or
// reflection, so scripts can neither read nor forge a pair; `#h in o` is the
// brand check. Every accessor passes the pair to Rust, which resolves it
// through the document on each call. Wrappers hold no pointers and keep no
// Rust node alive.
//
// Nothing here looks up a built-in at call time: the intrinsics used after
// install are captured below, the identity cache is a null-prototype
// object, and natives return only numbers and strings (an array built by
// Rust would be filled through index setters a script can install), so a script that replaces Object.create, Map or WeakMap methods,
// Function.prototype.call or Array index setters can't observe or change
// what the bindings do.
(function (n) {
  'use strict';
  const create = Object.create;
  const defineProperty = Object.defineProperty;
  const toString = String;
  const TypeErrorCtor = TypeError;

  // `new Brand(o, g, h)` returns `o` itself with the private fields added
  // (a base constructor that returns an object makes it the derived
  // class's `this`).
  class Stamp { constructor(o) { return o; } }
  class Brand extends Stamp {
    #g; #h;
    constructor(o, g, h) { super(o); this.#g = g; this.#h = h; }
    static slot(o) {
      if (typeof o !== 'object' || o === null || !(#h in o)) throw new TypeErrorCtor('Illegal invocation');
      return [o.#g, o.#h];
    }
  }
  const slot = Brand.slot;

  // Same node, same wrapper (DOM identity), within one document generation.
  let cacheGeneration = -1;
  let cache = create(null);

  function illegal() { throw new TypeErrorCtor('Illegal constructor'); }
  const Node = function Node() { illegal(); };
  const Element = function Element() { illegal(); };
  const Text = function Text() { illegal(); };
  const Comment = function Comment() { illegal(); };
  const Document = function Document() { illegal(); };
  Object.setPrototypeOf(Element.prototype, Node.prototype);
  Object.setPrototypeOf(Text.prototype, Node.prototype);
  Object.setPrototypeOf(Comment.prototype, Node.prototype);
  Object.setPrototypeOf(Document.prototype, Node.prototype);
  const protos = create(null);
  protos[1] = Element.prototype; protos[3] = Text.prototype; protos[8] = Comment.prototype; protos[9] = Document.prototype;
  const NodeProto = Node.prototype;

  function wrap(g, h) {
    if (h === 0) return null;
    if (g !== cacheGeneration) { cache = create(null); cacheGeneration = g; }
    let w = cache[h];
    if (w === undefined) {
      w = new Brand(create(protos[n.nodeType(g, h)] || NodeProto), g, h);
      cache[h] = w;
    }
    return w;
  }
  function getter(proto, name, f) {
    defineProperty(proto, name, { get: f, enumerable: true, configurable: true });
  }
  function method(proto, name, f) {
    defineProperty(proto, name, { value: f, writable: true, enumerable: true, configurable: true });
  }

  getter(Node.prototype, 'nodeType', function () { const s = slot(this); return n.nodeType(s[0], s[1]); });
  getter(Node.prototype, 'nodeName', function () { const s = slot(this); return n.nodeName(s[0], s[1]); });
  getter(Node.prototype, 'parentNode', function () {
    const s = slot(this); return wrap(s[0], n.parentNode(s[0], s[1]));
  });
  defineProperty(Node.prototype, 'textContent', {
    get: function () { const s = slot(this); return n.getTextContent(s[0], s[1]); },
    set: function (v) { const s = slot(this); n.setTextContent(s[0], s[1], v === null ? '' : toString(v)); },
    enumerable: true, configurable: true,
  });
  method(Node.prototype, 'appendChild', function (child) {
    const s = slot(this); const c = slot(child);
    n.appendChild(s[0], s[1], c[0], c[1]);
    return child;
  });
  getter(Element.prototype, 'id', function () {
    const s = slot(this); const v = n.getAttribute(s[0], s[1], 'id'); return v === null ? '' : v;
  });
  getter(Element.prototype, 'tagName', function () { const s = slot(this); return n.nodeName(s[0], s[1]); });
  method(Element.prototype, 'getAttribute', function (name) {
    const s = slot(this); return n.getAttribute(s[0], s[1], toString(name));
  });
  getter(Document.prototype, 'documentElement', function () {
    const s = slot(this); return wrap(s[0], n.documentElement(s[0], s[1]));
  });
  getter(Document.prototype, 'body', function () { const s = slot(this); return wrap(s[0], n.body(s[0], s[1])); });
  method(Document.prototype, 'getElementById', function (id) {
    const s = slot(this); return wrap(s[0], n.getElementById(s[0], s[1], toString(id)));
  });
  method(Document.prototype, 'createElement', function (name) {
    const s = slot(this); return wrap(s[0], n.createElement(s[0], s[1], toString(name)));
  });
  method(Document.prototype, 'createTextNode', function (data) {
    const s = slot(this); return wrap(s[0], n.createTextNode(s[0], s[1], toString(data)));
  });

  for (const [name, ctor] of Object.entries({ Node, Element, Text, Comment, Document })) {
    Object.defineProperty(globalThis, name, { value: ctor, writable: true, configurable: true });
  }
  // The loaded document, asked of Rust on every read: loading another
  // document needs no call into JavaScript.
  defineProperty(globalThis, 'document', {
    get: function () { const g = n.current(); return g === 0 ? null : wrap(g, n.currentRoot()); },
    enumerable: true, configurable: false,
  });
})
