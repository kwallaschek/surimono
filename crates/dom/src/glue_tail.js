
  // DOM §4.5: here because the interface numbers are in bindings.js.
  for (const [name, id] of [
    ['beforeunloadevent', I_BeforeUnloadEvent], ['compositionevent', I_CompositionEvent],
    ['customevent', I_CustomEvent], ['devicemotionevent', I_DeviceMotionEvent],
    ['deviceorientationevent', I_DeviceOrientationEvent], ['dragevent', I_DragEvent],
    ['event', I_Event], ['events', I_Event], ['focusevent', I_FocusEvent],
    ['hashchangeevent', I_HashChangeEvent], ['htmlevents', I_Event],
    ['keyboardevent', I_KeyboardEvent], ['messageevent', I_MessageEvent],
    ['mouseevent', I_MouseEvent], ['mouseevents', I_MouseEvent], ['storageevent', I_StorageEvent],
    ['svgevents', I_Event], ['textevent', I_TextEvent], ['touchevent', I_TouchEvent],
    ['uievent', I_UIEvent], ['uievents', I_UIEvent],
  ]) eventNames[name] = id;

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
  // HTML §3.1.1: the name is kept for old pages, and is Document's.
  defineProperty(G, 'HTMLDocument', { value: ctors[I_Document], writable: true, configurable: true });
  // [LegacyUnforgeable] `window` and `document`; [Replaceable] `self`.
  defineProperty(G, 'window', { value: G, enumerable: true, configurable: false });
  defineProperty(G, 'self', { value: G, writable: true, enumerable: true, configurable: true });
  // The loaded document, asked of Rust on every read: loading another
  // document needs no call into JavaScript.
  defineProperty(G, 'document', {
    get: function () { const g = current(); return g === 0 ? null : wrap(g, root()); },
    enumerable: true, configurable: false,
  });

  // The rest of the Window-reflecting set: what only Window has.
  for (const name in handlerNames[I_Window]) {
    if (handlerNames[I_HTMLElement][name] !== true) reflected[name] = true;
  }

  // A handler given to the event loop reports what it throws through the
  // window's `error` event, like a listener: the prelude's own report goes
  // straight to the caller. HTML §8.7 (timers), §8.8 (queueMicrotask) and
  // §8.10.1 (frames) all "report the exception". `expose` keeps each
  // function's shape: what the prelude's has, down to `length`.
  function expose(name, f, length) {
    defineProperty(f, 'length', { value: length, configurable: true });
    defineProperty(G, name, { value: f, writable: true, enumerable: true, configurable: true });
  }
  function setTimer(real, a) {
    const argc = a.length;
    if (argc === 0) return real();
    // Converted here, in argument order, as the prelude would have.
    let handler = a[0];
    const isFunction = typeof handler === 'function';
    if (!isFunction) handler = `${handler}`;
    const args = create(null);
    args.length = argc > 2 ? argc - 2 : 0;
    for (let i = 2; i < argc; i++) args[i - 2] = a[i];
    const task = function () {
      try {
        if (isFunction) apply(handler, G, args);
        else n.runSource(apply(toWellFormed, handler, []));
      } catch (x) {
        reportException(x);
      }
    };
    return argc > 1 ? real(task, a[1]) : real(task);
  }
  expose('setTimeout', {
    setTimeout() { return setTimer(setTimeoutReal, arguments); },
  }.setTimeout, 1);
  expose('setInterval', {
    setInterval() { return setTimer(setIntervalReal, arguments); },
  }.setInterval, 1);
  expose('requestAnimationFrame', {
    requestAnimationFrame(callback) {
      if (typeof callback !== 'function') return requestFrameReal(callback);
      return requestFrameReal(function (time) {
        try {
          apply(callback, undefined, [time]);
        } catch (x) {
          reportException(x);
        }
      });
    },
  }.requestAnimationFrame, 1);
  expose('queueMicrotask', {
    queueMicrotask(callback) {
      if (typeof callback !== 'function') return queueMicrotaskReal(callback);
      queueMicrotaskReal(function () {
        try {
          apply(callback, undefined, []);
        } catch (x) {
          reportException(x);
        }
      });
    },
  }.queueMicrotask, 1);

  // Timer ids up to this one are the page's lifecycle, and no script may
  // cancel those (`start`).
  let lifecycleLast = 0;
  function clearTimer(a) {
    if (a.length === 0) return;
    const id = a[0] | 0;
    if (id < 1 || id > lifecycleLast) clearTimeoutReal(id);
  }
  expose('clearTimeout', { clearTimeout() { clearTimer(arguments); } }.clearTimeout, 0);
  expose('clearInterval', { clearInterval() { clearTimer(arguments); } }.clearInterval, 0);

  // HTML §8.1.3.5: the running script's element, for the length of its run.
  let currentScript = null;
  attr(I_Document, 'currentScript', function () {
    self(this, I_Document);
    return currentScript;
  });

  // One script's run, with its element as `currentScript`; Rust has the
  // source. Returns the rest of its turn, for a step of its own: HTML runs the
  // microtask checkpoint inside "run a classic script", so the script's
  // microtasks, however they chain, still see its element and come before
  // its load or error event.
  function runScript(i, g, h) {
    const element = wrap(g, h);
    let ok = true;
    if (n.scriptFailed(i)) {
      ok = false;
    } else {
      currentScript = element;
      try {
        n.runScript(i);
      } catch (x) {
        reportException(x);
      }
    }
    return () => {
      currentScript = null;
      if (n.scriptExternal(i)) fire(stateOf(element), ok ? 'load' : 'error');
    };
  }

  // Every step of the page's life is queued here, before any of its scripts
  // runs: same due time, earlier sequence numbers, so they all come before
  // the page's own timers, in this order, each a task with a microtask
  // checkpoint after it. All of them are timers so that one call to the
  // event loop, under one time limit, runs the page.
  //
  // Two things keep them from being a page's timers. Their ids are refused
  // by clearTimeout and clearInterval, so a page's clear-all loop can't
  // cancel them. And each does its work in a microtask of its task: the
  // loop has left the timer by then, so a timer a script sets starts from
  // nesting level 0, as a parser's script does in a browser (HTML §8.7).
  return function start(g) {
    const setState = (state) => () => {
      call(OP_setReadyState, g, root(), state);
      fire(documentState(), 'readystatechange');
    };
    const step = (f) => {
      lifecycleLast = setTimeoutReal(() => queueMicrotaskReal(f), 0);
    };
    const turn = (at) => {
      let finish;
      step(() => { finish = runScript(at, g, n.scriptNode(at)); });
      step(() => { finish(); });
    };
    const scripts = n.scriptCount();
    for (let i = 0; i < scripts; i++) {
      if (!n.scriptDeferred(i)) turn(i);
    }
    step(setState(1));
    for (let i = 0; i < scripts; i++) {
      if (n.scriptDeferred(i)) turn(i);
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
