// The JavaScript half of the DOM bindings. Called once per context with the
// native functions; returns nothing and leaves no way to reach them.
//
// A wrapper is an ordinary object whose [generation, handle] pair lives in a
// closure-private WeakMap, so scripts can neither read nor forge it. Every
// accessor passes the pair to Rust, which resolves it through the document
// on each call. Wrappers hold no pointers and keep no Rust node alive.
(function (n) {
  'use strict';
  const slots = new WeakMap();
  // Same node, same wrapper (DOM identity), within one document generation.
  let cacheGeneration = -1;
  let cache = new Map();

  function illegal() { throw new TypeError('Illegal constructor'); }
  const Node = function Node() { illegal(); };
  const Element = function Element() { illegal(); };
  const Text = function Text() { illegal(); };
  const Comment = function Comment() { illegal(); };
  const Document = function Document() { illegal(); };
  Object.setPrototypeOf(Element.prototype, Node.prototype);
  Object.setPrototypeOf(Text.prototype, Node.prototype);
  Object.setPrototypeOf(Comment.prototype, Node.prototype);
  Object.setPrototypeOf(Document.prototype, Node.prototype);
  const protos = { 1: Element.prototype, 3: Text.prototype, 8: Comment.prototype, 9: Document.prototype };

  function wrap(g, h) {
    if (h === 0) return null;
    if (g !== cacheGeneration) { cache = new Map(); cacheGeneration = g; }
    let w = cache.get(h);
    if (w === undefined) {
      w = Object.create(protos[n.nodeType(g, h)] || Node.prototype);
      slots.set(w, [g, h]);
      cache.set(h, w);
    }
    return w;
  }
  function slot(w) {
    const s = slots.get(w);
    if (s === undefined) throw new TypeError('Illegal invocation');
    return s;
  }
  function getter(proto, name, f) {
    Object.defineProperty(proto, name, { get: f, enumerable: true, configurable: true });
  }
  function method(proto, name, f) {
    Object.defineProperty(proto, name, { value: f, writable: true, enumerable: true, configurable: true });
  }

  getter(Node.prototype, 'nodeType', function () { const s = slot(this); return n.nodeType(s[0], s[1]); });
  getter(Node.prototype, 'nodeName', function () { const s = slot(this); return n.nodeName(s[0], s[1]); });
  getter(Node.prototype, 'parentNode', function () {
    const s = slot(this); return wrap(s[0], n.parentNode(s[0], s[1]));
  });
  Object.defineProperty(Node.prototype, 'textContent', {
    get: function () { const s = slot(this); return n.getTextContent(s[0], s[1]); },
    set: function (v) { const s = slot(this); n.setTextContent(s[0], s[1], v === null ? '' : String(v)); },
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
    const s = slot(this); return n.getAttribute(s[0], s[1], String(name));
  });
  getter(Document.prototype, 'documentElement', function () {
    const s = slot(this); return wrap(s[0], n.documentElement(s[0], s[1]));
  });
  getter(Document.prototype, 'body', function () { const s = slot(this); return wrap(s[0], n.body(s[0], s[1])); });
  method(Document.prototype, 'getElementById', function (id) {
    const s = slot(this); return wrap(s[0], n.getElementById(s[0], s[1], String(id)));
  });
  method(Document.prototype, 'createElement', function (name) {
    const s = slot(this); return wrap(s[0], n.createElement(s[0], s[1], String(name)));
  });
  method(Document.prototype, 'createTextNode', function (data) {
    const s = slot(this); return wrap(s[0], n.createTextNode(s[0], s[1], String(data)));
  });

  for (const [name, ctor] of Object.entries({ Node, Element, Text, Comment, Document })) {
    Object.defineProperty(globalThis, name, { value: ctor, writable: true, configurable: true });
  }
  // The loaded document, asked of Rust on every read: loading another
  // document needs no call into JavaScript.
  Object.defineProperty(globalThis, 'document', {
    get: function () { const d = n.current(); return d === null ? null : wrap(d[0], d[1]); },
    enumerable: true, configurable: false,
  });
})
