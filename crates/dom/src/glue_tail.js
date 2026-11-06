
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
  windowState.m = brands[I_Window]; windowState.g = 0; windowState.h = 0; windowState.w = G;
  new Brand(G, windowState);
  setPrototypeOf(G, protos[I_Window]);
  // The engine's own tag would hide Window.prototype's.
  rdelete(G, symbolTag);

  for (const i in ctors) {
    defineProperty(G, names[i], { value: ctors[i], writable: true, configurable: true });
  }
  // HTML §7.2.2: the name is kept for old pages, and is Document's.
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

  // The sandbox's `performance`, with the brand HR Time gives it: an event
  // target, whose `now` stays the prelude's virtual clock.
  if (performanceObject !== undefined) {
    const p = create(null);
    p.m = brands[I_Performance]; p.g = 0; p.h = 0; p.w = performanceObject;
    new Brand(performanceObject, p);
    setPrototypeOf(performanceObject, protos[I_Performance]);
  }

  // HTML §7.3.3 named access on the Window object, as WebIDL §3.7.4's named
  // properties object: between Window.prototype and EventTarget.prototype, so
  // anything a script defines hides a name. Not a Proxy, as `exotic` is: the
  // engine reads a global through the prototype chain, and a Proxy there
  // would turn a ReferenceError into undefined. So the object gets a getter
  // for each `id` of the parsed tree before the first script (`start`); what
  // a script adds later has no name here. At most MAX_WINDOW_NAMES.
  const windowProperties = create(protos[I_EventTarget]);
  setPrototypeOf(protos[I_Window], windowProperties);
  const MAX_WINDOW_NAMES = 4096;
  function nameElements(g) {
    let count = 0;
    nameList(call(OP_idNames, g, root()), (name) => {
      if (name === '' || count === MAX_WINDOW_NAMES || rhas(G, name)) return;
      count++;
      defineProperty(windowProperties, name, {
        get: function () {
          const r = current() === g ? wrap(g, call(OP_getElementById, g, root(), name)) : null;
          return r === null ? undefined : r;
        },
        // WebIDL §3.7.4 reports a writable data property, so an assignment
        // makes an own property of the window, as for `chart = init(...)`.
        set: function (v) { replaceable(G, name, v); },
        enumerable: false, configurable: true,
      });
    });
  }

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

  // HTML §8.5.3. No state: all of it is the argument.
  H.new_DOMParser = (target) => make(I_DOMParser, target).w;
  H.DOMParser_parseFromString = (s, string, type) => {
    const text = str(string), t = `${type}`;
    if (t !== 'text/html' && t !== 'text/xml' && t !== 'application/xml' &&
        t !== 'application/xhtml+xml' && t !== 'image/svg+xml') {
      throw new TE('parseFromString: the type is not a DOMParserSupportedType');
    }
    const g = current();
    const h = s.doc === undefined ? root() : s.doc.h;
    return wrap(g, call(OP_parseFromString, g, h, text, t));
  };
  H.new_XMLSerializer = (target) => make(I_XMLSerializer, target).w;

  // DOM §5.4. WebIDL §3.2.18: a dictionary's members in lexicographic order.
  H.new_StaticRange = (target, init) => {
    if (init === null || init === undefined || (typeof init !== 'object' && typeof init !== 'function')) {
      throw new TE('StaticRangeInit is required');
    }
    const member = (k) => {
      const v = init[k];
      if (v === undefined) throw new TE(`StaticRangeInit.${k} is required`);
      return v;
    };
    const g = current();
    const endContainer = obj(member('endContainer'), I_Node, g);
    const endOffset = member('endOffset') >>> 0;
    const startContainer = obj(member('startContainer'), I_Node, g);
    const startOffset = member('startOffset') >>> 0;
    const h = call(OP_staticRange, g, root(), startContainer, startOffset, endContainer, endOffset);
    return adopt(wrap(g, h), target, I_StaticRange);
  };
  H.XMLSerializer_serializeToString = (s, node) => call(OP_serializeXml, node.g, node.h, node.h);

  // Timer ids up to this one are the page's lifecycle, and no script may
  // cancel those (`start`), nor the glue's own tasks (`task`).
  let lifecycleLast = 0;
  function clearTimer(a) {
    if (a.length === 0) return;
    const id = a[0] | 0;
    if ((id < 1 || id > lifecycleLast) && glueTasks[id] !== true) clearTimeoutReal(id);
  }
  expose('clearTimeout', { clearTimeout() { clearTimer(arguments); } }.clearTimeout, 0);
  expose('clearInterval', { clearInterval() { clearTimer(arguments); } }.clearInterval, 0);

  // HTML §3.1.1 and §7.2: only the page's document has a browsing context.
  const isPage = (s) => s.g === current() && s.h === root();
  // A frame has no realm or scripts of its own: its window is an object over
  // the page's, with its own `document`. Enough for what looks things up on
  // `contentWindow` or `defaultView`, and no way to run anything.
  function frameWindow(s) {
    if (s.frameWindow === undefined) {
      const w = create(G);
      const d = create(null);
      d.value = s.w; d.enumerable = true;
      defineProperty(w, 'document', d);
      own(w, 'location', locationOf(s));
      own(w, 'history', historyRecord(s).w);
      own(w, 'DOMParser', frameParser(s));
      s.frameWindow = w;
    }
    return s.frameWindow;
  }
  // A DOMParser's URL is its "associated document"'s, the document of the
  // global it was made in (HTML §8.5.3), and that is what a frame's window
  // differs in: its own constructor, on the one prototype.
  function frameParser(d) {
    const C = function DOMParser() {
      if (new.target === undefined) throw new TE('Constructor requires new');
      const p = make(I_DOMParser, new.target === C ? ctors[I_DOMParser] : new.target);
      p.doc = d;
      return p.w;
    };
    C.prototype = protos[I_DOMParser];
    return C;
  }
  attr(I_Document, 'defaultView', function () {
    const s = self(this, I_Document);
    if (isPage(s)) return G;
    return call(OP_isFrameDocument, s.g, s.h) ? frameWindow(s) : null;
  });
  attr(I_HTMLIFrameElement, 'contentWindow', function () {
    const d = stateOf(this.contentDocument);
    return d === null ? null : frameWindow(d);
  });
  // Frames whose source Rust has, one parse per call, under this call's limits.
  function parseFrames() {
    const g = current();
    while (call(OP_loadFrame, g, root()));
  }
  function frameLoads() {
    const g = current();
    parseFrames();
    for (;;) {
      const h = call(OP_nextFrameLoad, g, root());
      if (h === 0) return;
      fire(stateOf(wrap(g, h)), 'load');
    }
  }
  // Called by `run_page` when frames have loaded. A script can call it too,
  // which fires only what was due.
  defineProperty(G, '__surimonoFrameLoads', { value: frameLoads });
  // HTML §7.2.2.3 "document-tree child navigables": the page's iframes.
  // `frames` is the window in a browser; here it is a list over it, built per
  // read, since the global object has no indexed properties.
  const pageByTag = protos[I_Document].getElementsByTagName;
  function childWindows() {
    const out = create(G);
    const d = current() === 0 ? null : wrap(current(), root());
    let n = 0;
    if (d !== null) {
      const list = apply(pageByTag, d, ['iframe']);
      const m = list.length;
      for (let i = 0; i < m; i++) {
        const w = list[i].contentWindow;
        if (w !== null) {
          const e = create(null);
          e.value = w; e.enumerable = true;
          defineProperty(out, n++, e);
        }
      }
    }
    const l = create(null);
    l.value = n;
    defineProperty(out, 'length', l);
    return out;
  }
  attr(I_Window, 'frames', function () { self(this, I_Window); return childWindows(); });
  attr(I_Window, 'length', function () { self(this, I_Window); return childWindows().length; });
  attr(I_Document, 'location', function () {
    if (!isPage(self(this, I_Document))) return null;
    return H.Window_location();
  }, function (v) {
    if (isPage(self(this, I_Document))) H.Window_location().href = v;
  });

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
    // HTML "execute the script element": moved to another document, it's over.
    if (!call(OP_inPage, g, h)) return () => { currentScript = null; };
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
      if (n.scriptExternal(i)) fireInTask(stateOf(element), ok ? 'load' : 'error');
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
    nameElements(g);
    const setState = (state) => () => {
      call(OP_setReadyState, g, root(), state);
      fireInTask(documentState(), 'readystatechange');
    };
    const step = (f) => {
      lifecycleLast = setTimeoutReal(() => queueMicrotaskReal(f), 0);
    };
    const turn = (at) => {
      let finish;
      step(() => { finish = runScript(at, g, n.scriptNode(at)); });
      step(() => { finish(); });
    };
    // The parser's frames, before any script reads them.
    step(parseFrames);
    const scripts = n.scriptCount();
    for (let i = 0; i < scripts; i++) {
      if (!n.scriptDeferred(i)) turn(i);
    }
    step(setState(1));
    for (let i = 0; i < scripts; i++) {
      if (n.scriptDeferred(i)) turn(i);
    }
    step(() => { fireInTask(documentState(), 'DOMContentLoaded', true); });
    step(setState(2));
    // The frames the parser found: `load` waits for them (HTML §4.8.5).
    step(frameLoads);
    step(() => {
      // HTML: `load` goes to the window, with the document as its target.
      const s = make(I_Event);
      initEvent(s, 'load', false, false);
      s.isTrusted = true;
      s.alone = true;
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
