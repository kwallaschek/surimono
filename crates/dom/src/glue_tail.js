
  const documentState = () => stateOf(wrap(current(), root()));

  // The global object is the window (HTML §7.2): it gets its brand and
  // inherits from Window.prototype, so `addEventListener` is a global.
  windowState = create(null);
  windowState.m = masks[I_Window]; windowState.g = 0; windowState.h = 0; windowState.w = G;
  new Brand(G, windowState);
  setPrototypeOf(G, protos[I_Window]);
  // The engine's own tag would hide Window.prototype's.
  rdelete(G, symbolTag);

  for (const i in ctors) {
    defineProperty(G, names[i], { value: ctors[i], writable: true, configurable: true });
  }
  // [LegacyUnforgeable] `window` and `document`; [Replaceable] `self`.
  defineProperty(G, 'window', { value: G, enumerable: true, configurable: false });
  defineProperty(G, 'self', { value: G, writable: true, enumerable: true, configurable: true });
  // The loaded document, asked of Rust on every read: loading another
  // document needs no call into JavaScript.
  defineProperty(G, 'document', {
    get: function () { const g = current(); return g === 0 ? null : wrap(g, root()); },
    enumerable: true, configurable: false,
  });

  // One script's turn. `at` is the node handle, for its load or error event
  // and `currentScript`; Rust has the source.
  function runScript(i, g, h) {
    const element = wrap(g, h);
    let ok = true;
    if (n.scriptFailed(i)) {
      ok = false;
    } else {
      try {
        n.runScript(i);
      } catch (x) {
        reportException(x);
      }
    }
    if (n.scriptExternal(i)) fire(stateOf(element), ok ? 'load' : 'error');
  }

  // Every step of the page's life is queued here, before any of its scripts
  // runs: same due time, earlier sequence numbers, so they all come before
  // the page's own timers, in this order, each a task with a microtask
  // checkpoint after it.
  return function start(g) {
    const setState = (state) => () => {
      call(OP_setReadyState, g, root(), state);
      fire(documentState(), 'readystatechange');
    };
    const step = (f) => schedule(f, 0);
    const scripts = n.scriptCount();
    for (let i = 0; i < scripts; i++) {
      const at = i;
      if (!n.scriptDeferred(i)) step(() => runScript(at, g, n.scriptNode(at)));
    }
    step(setState(1));
    for (let i = 0; i < scripts; i++) {
      const at = i;
      if (n.scriptDeferred(i)) step(() => runScript(at, g, n.scriptNode(at)));
    }
    step(() => { fire(documentState(), 'DOMContentLoaded', true); });
    step(setState(2));
    step(() => {
      // HTML: `load` goes to the window, with the document as its target.
      const s = make(I_Event);
      initEvent(s, 'load', false, false);
      s.isTrusted = true;
      s.dispatching = true;
      s.target = documentState().w;
      const path = list();
      add(path, windowState);
      s.path = path;
      s.eventPhase = 2;
      invoke(windowState, s, true);
      invoke(windowState, s, false);
      s.eventPhase = 0;
      s.currentTarget = null;
      s.path = undefined;
      s.dispatching = false;
    });
  };
})
