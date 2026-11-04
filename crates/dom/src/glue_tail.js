
  for (const i in ctors) {
    defineProperty(globalThis, names[i], { value: ctors[i], writable: true, configurable: true });
  }
  // The loaded document, asked of Rust on every read: loading another
  // document needs no call into JavaScript.
  defineProperty(globalThis, 'document', {
    get: function () { const g = current(); return g === 0 ? null : wrap(g, root()); },
    enumerable: true, configurable: false,
  });
})
