  // Events and page lifecycle. Part of the function glue_head.js opens.
  //
  // Listeners are JavaScript values, so they stay here, under the memory
  // limit: in the target's private record, with the fields of an event in its
  // own. `H` holds the bodies of the members the generator declares for the
  // interfaces in JS_INTERFACES (crates/webidl/src/bindgen.rs).
  //
  // Lists are null-prototype objects with a count, not arrays: a script's
  // index setters on Array.prototype must see nothing of them.
  function list() { const l = create(null); l.n = 0; return l; }
  function add(l, x) { l[l.n++] = x; }
  function copy(l) { const c = list(); for (let i = 0; i < l.n; i++) add(c, l[i]); return c; }
  function realArray(l) {
    const out = [];
    for (let i = 0; i < l.n; i++) {
      const d = create(null);
      d.value = l[i]; d.writable = true; d.enumerable = true; d.configurable = true;
      defineProperty(out, i, d);
    }
    return out;
  }

  // A new wrapper of a hand-written interface. `target` is new.target.
  function make(id, target) {
    const s = create(null);
    s.m = masks[id]; s.g = 0; s.h = 0;
    const w = new Brand(create(protos[id]), s);
    s.w = w;
    if (target !== undefined) adopt(w, target, id);
    return s;
  }

  // WebIDL dictionary conversion (§3.2.17): undefined and null are the empty
  // dictionary; other non-objects are a TypeError.
  function dictionary(d, what) {
    if (d === undefined || d === null) return create(null);
    if (typeof d !== 'object' && typeof d !== 'function') {
      throw new TE(what + ': the argument is not a dictionary');
    }
    return d;
  }

  // ------ EventTarget (DOM §2.7) ------

  H.new_EventTarget = (target) => make(I_EventTarget, target).w;

  // WebIDL's `(EventListenerOptions or boolean)`: a function is an object,
  // so it is a dictionary.
  const isObject = (x) => (typeof x === 'object' && x !== null) || typeof x === 'function';
  function flatten(options) { return isObject(options) ? !!options.capture : !!options; }
  function listenersOf(s, create_) {
    if (s.l === undefined && create_) s.l = list();
    return s.l;
  }
  H.EventTarget_addEventListener = (s, type, callback, options) => {
    // §2.7.3 "flatten more". `signal` isn't read: AbortSignal isn't bound.
    const capture = flatten(options);
    let once = false, passive = false;
    if (isObject(options)) {
      once = !!options.once;
      passive = !!options.passive;
    }
    if (callback === null || callback === undefined) return;
    if (typeof callback !== 'object' && typeof callback !== 'function') {
      throw new TE('addEventListener: the listener is not an object');
    }
    syncHandlers(s);
    const l = listenersOf(s, true);
    for (let i = 0; i < l.n; i++) {
      const e = l[i];
      if (e.type === type && e.callback === callback && e.capture === capture) return;
    }
    addListener(l, type, callback, capture, once, passive);
  };
  function addListener(l, type, callback, capture, once, passive) {
    const e = create(null);
    e.type = type; e.callback = callback; e.capture = capture;
    e.once = once; e.passive = passive; e.removed = false;
    add(l, e);
    return e;
  }
  function removeAt(l, at) {
    l[at].removed = true;
    for (let i = at + 1; i < l.n; i++) l[i - 1] = l[i];
    delete l[--l.n];
  }
  H.EventTarget_removeEventListener = (s, type, callback, options) => {
    const capture = flatten(options);
    const l = listenersOf(s, false);
    if (l === undefined) return;
    for (let i = 0; i < l.n; i++) {
      const e = l[i];
      if (e.type === type && e.callback === callback && e.capture === capture) {
        removeAt(l, i);
        return;
      }
    }
  };
  H.EventTarget_dispatchEvent = (s, event) => {
    if (event.dispatching || !event.initialized) {
      throwDom('InvalidStateError', 'dispatchEvent: the event is being dispatched or not initialized');
    }
    event.isTrusted = false;
    return dispatch(s, event);
  };

  // §2.10 "get the parent": a node's parent; the document's is the window,
  // except for `load` (HTML §13.2.7: the event's target stays the document
  // and the window's listeners get it, but it doesn't come from the tree).
  function parentOf(s, event) {
    if (!is(s, I_Node)) return null;
    const p = wrap(s.g, call(OP_parentNode, s.g, s.h));
    if (p !== null) return stateOf(p);
    if (s.h === root() && s.g === current() && event.type !== 'load') return windowState;
    return null;
  }

  // §2.9 "dispatch". No shadow trees, so no retargeting and no slots.
  function dispatch(target, event) {
    event.dispatching = true;
    event.target = target.w;
    const path = list();
    for (let at = target; at !== null; at = parentOf(at, event)) add(path, at);
    event.path = path;
    for (let i = path.n - 1; i >= 0; i--) {
      event.eventPhase = i === 0 ? 2 : 1;
      invoke(path[i], event, true);
    }
    for (let i = 0; i < path.n; i++) {
      if (i !== 0 && !event.bubbles) continue;
      event.eventPhase = i === 0 ? 2 : 3;
      invoke(path[i], event, false);
    }
    event.eventPhase = 0;
    event.currentTarget = null;
    event.path = undefined;
    event.dispatching = false;
    event.stop = false;
    event.stopNow = false;
    return !event.canceled;
  }

  // §2.9 "invoke" and "inner invoke". A listener that throws is reported and
  // the others still run.
  function invoke(target, event, capturing) {
    if (event.stop) return;
    event.currentTarget = target.w;
    syncHandlers(target);
    const all = target.l;
    if (all === undefined) return;
    const l = copy(all);
    for (let i = 0; i < l.n; i++) {
      const e = l[i];
      if (e.removed || e.type !== event.type || e.capture !== capturing) continue;
      if (e.once) {
        for (let k = 0; k < all.n; k++) if (all[k] === e) { removeAt(all, k); break; }
      }
      event.passive = e.passive;
      try {
        const cb = e.callback;
        if (typeof cb === 'function') {
          apply(cb, event.currentTarget, [event.w]);
        } else {
          const f = cb.handleEvent;
          if (typeof f !== 'function') throw new TE('the listener has no handleEvent method');
          apply(f, cb, [event.w]);
        }
      } catch (x) {
        reportException(x);
      }
      event.passive = false;
      if (event.stopNow) break;
    }
  }

  // ------ Event (DOM §2.2) ------

  // §2.5 "initialize": what `initEvent` resets; it leaves `composed` and
  // `timeStamp` alone.
  function initialize(s, type, bubbles, cancelable) {
    s.initialized = true; s.stop = false; s.stopNow = false; s.canceled = false;
    s.isTrusted = false; s.target = null;
    s.type = type; s.bubbles = bubbles; s.cancelable = cancelable;
  }
  // A new event's fields.
  function initEvent(s, type, bubbles, cancelable) {
    initialize(s, type, bubbles, cancelable);
    s.composed = false; s.dispatching = false; s.passive = false;
    s.currentTarget = null; s.eventPhase = 0;
    s.timeStamp = now();
  }
  function eventInit(s, type, d, what) {
    d = dictionary(d, what);
    const bubbles = !!d.bubbles, cancelable = !!d.cancelable, composed = !!d.composed;
    initEvent(s, type, bubbles, cancelable);
    s.composed = composed;
    return d;
  }
  H.new_Event = (target, type, d) => {
    const s = make(I_Event, target);
    eventInit(s, type, d, 'Event');
    return s.w;
  };
  H.Event_type = (s) => s.type;
  H.Event_target = (s) => s.target;
  H.Event_srcElement = (s) => s.target;
  H.Event_currentTarget = (s) => s.currentTarget;
  H.Event_composedPath = (s) => {
    const l = list();
    if (s.path !== undefined) for (let i = 0; i < s.path.n; i++) add(l, s.path[i].w);
    return realArray(l);
  };
  H.Event_eventPhase = (s) => s.eventPhase;
  H.Event_stopPropagation = (s) => { s.stop = true; };
  H.Event_cancelBubble = (s) => s.stop;
  H.set_Event_cancelBubble = (s, v) => { if (v) s.stop = true; };
  H.Event_stopImmediatePropagation = (s) => { s.stop = true; s.stopNow = true; };
  H.Event_bubbles = (s) => s.bubbles;
  H.Event_cancelable = (s) => s.cancelable;
  function cancel(s) { if (s.cancelable && !s.passive) s.canceled = true; }
  H.Event_returnValue = (s) => !s.canceled;
  H.set_Event_returnValue = (s, v) => { if (!v) cancel(s); };
  H.Event_preventDefault = cancel;
  H.Event_defaultPrevented = (s) => s.canceled;
  H.Event_composed = (s) => s.composed;
  H.Event_isTrusted = (s) => s.isTrusted;
  H.Event_timeStamp = (s) => s.timeStamp;
  H.Event_initEvent = (s, type, bubbles, cancelable) => {
    if (!s.dispatching) initialize(s, type, bubbles, cancelable);
  };

  H.new_CustomEvent = (target, type, d) => {
    const s = make(I_CustomEvent, target);
    const init = eventInit(s, type, d, 'CustomEvent');
    const detail = init.detail;
    s.detail = detail === undefined ? null : detail;
    return s.w;
  };
  H.CustomEvent_detail = (s) => s.detail;
  H.CustomEvent_initCustomEvent = (s, type, bubbles, cancelable, detail) => {
    if (s.dispatching) return;
    initialize(s, type, bubbles, cancelable);
    s.detail = detail === undefined ? null : detail;
  };

  H.new_ErrorEvent = (target, type, d) => {
    const s = make(I_ErrorEvent, target);
    const init = eventInit(s, type, d, 'ErrorEvent');
    const message = init.message, filename = init.filename;
    s.message = message === undefined ? '' : str(message);
    s.filename = filename === undefined ? '' : str(filename);
    s.lineno = init.lineno >>> 0;
    s.colno = init.colno >>> 0;
    s.error = init.error;
    return s.w;
  };
  H.ErrorEvent_message = (s) => s.message;
  H.ErrorEvent_filename = (s) => s.filename;
  H.ErrorEvent_lineno = (s) => s.lineno;
  H.ErrorEvent_colno = (s) => s.colno;
  H.ErrorEvent_error = (s) => s.error;

  H.Window_devicePixelRatio = () => 1;

  // ------ MutationObserver (DOM §4.3) ------
  //
  // Registrations and records are in Rust, which sees the mutations; the
  // callback is here. Once an observer exists, every native call goes through
  // `observedCall`, which queues the notify microtask when a record is
  // waiting. A page without observers pays nothing.

  // Observer numbers belong to one document, as handles do: an observer
  // carries its generation (`og`) and only its own document's list has it,
  // so loading another neither reaches old observers nor keeps them.
  let observers = list();
  let observersGeneration = 0;
  function liveObservers() {
    const g = current();
    if (g !== observersGeneration) { observers = list(); observersGeneration = g; }
    return observers;
  }
  let notifyQueued = false;
  const nativeCall = n.call;
  function observedCall() {
    const r = apply(nativeCall, undefined, arguments);
    if (!notifyQueued && nativeCall(OP_recordsPending, current(), root())) {
      notifyQueued = true;
      queueMicrotaskReal(notify);
    }
    return r;
  }
  function takeRecords(s) {
    const records = list();
    const g = current();
    for (;;) {
      const h = call(OP_observerTake, g, root(), s.observer);
      if (h === 0) break;
      add(records, wrap(g, h));
    }
    return records;
  }
  // §4.3.2 "notify mutation observers".
  function notify() {
    notifyQueued = false;
    const all = copy(liveObservers());
    for (let i = 0; i < all.n; i++) {
      const s = all[i];
      const records = takeRecords(s);
      if (records.n === 0) continue;
      try {
        apply(s.callback, s.w, [realArray(records), s.w]);
      } catch (x) {
        reportException(x);
      }
    }
  }
  H.new_MutationObserver = (target, callback) => {
    if (typeof callback !== 'function') throw new TE('MutationObserver: the callback is not a function');
    const s = make(I_MutationObserver, target);
    s.callback = callback;
    s.og = current();
    s.observer = call(OP_observerNew, s.og, root());
    add(liveObservers(), s);
    call = observedCall;
    return s.w;
  };
  // §4.3.1 observe.
  H.MutationObserver_observe = (s, target, d) => {
    if (s.og !== current()) stale();
    d = dictionary(d, 'observe');
    // In WebIDL's order for a dictionary: lexicographic.
    const filterGiven = d.attributeFilter;
    const filter = filterGiven === undefined ? undefined : stringSequence(filterGiven, 'attributeFilter');
    const attributeOld = d.attributeOldValue;
    let attributes = d.attributes, characterData = d.characterData;
    const characterOld = d.characterDataOldValue;
    const childList = !!d.childList, subtree = !!d.subtree;
    if (attributes === undefined && (attributeOld !== undefined || filter !== undefined)) attributes = true;
    if (characterData === undefined && characterOld !== undefined) characterData = true;
    attributes = !!attributes; characterData = !!characterData;
    if (!childList && !attributes && !characterData) {
      throw new TE('observe: one of childList, attributes or characterData is required');
    }
    if (!!attributeOld && !attributes) throw new TE('observe: attributeOldValue needs attributes');
    if (filter !== undefined && !attributes) throw new TE('observe: attributeFilter needs attributes');
    if (!!characterOld && !characterData) throw new TE('observe: characterDataOldValue needs characterData');
    let names = '';
    if (filter !== undefined) {
      for (let i = 0; i < filter.n; i++) names += (i === 0 ? '' : '\n') + filter[i];
    }
    const flags = (childList ? 1 : 0) | (attributes ? 2 : 0) | (characterData ? 4 : 0) | (subtree ? 8 : 0) |
      (attributeOld ? 16 : 0) | (characterOld ? 32 : 0) | (filter !== undefined ? 64 : 0);
    if (target.g !== current()) stale();
    call(OP_observe, target.g, target.h, s.observer, target.h, flags, names);
  };
  // WebIDL §3.2.21 `sequence<DOMString>`, as a list.
  function stringSequence(x, what) {
    if (!isObject(x)) throw new TE(what + ': not a sequence');
    const method = x[symbolIterator];
    if (typeof method !== 'function') throw new TE(what + ': not iterable');
    const it = apply(method, x, []);
    if (!isObject(it)) throw new TE(what + ': the iterator is not an object');
    const next = it.next;
    const out = list();
    for (;;) {
      const r = apply(next, it, []);
      if (!isObject(r)) throw new TE(what + ': the iterator result is not an object');
      if (r.done) return out;
      add(out, str(r.value));
    }
  }
  H.MutationObserver_disconnect = (s) => {
    if (s.og !== current()) stale();
    call(OP_observerDisconnect, current(), root(), s.observer);
  };
  H.MutationObserver_takeRecords = (s) => {
    if (s.og !== current()) stale();
    return realArray(takeRecords(s));
  };

  // ------ Firing, and the page's life (HTML §13.2.7 "the end") ------

  // §2.10 "fire an event".
  function fire(target, type, bubbles, cancelable) {
    const s = make(I_Event);
    initEvent(s, type, !!bubbles, !!cancelable);
    s.isTrusted = true;
    return dispatch(target, s);
  }

  // HTML §8.1.6.2 "report an exception": an `error` event at the window,
  // and the exception counted for the caller unless a listener cancels it.
  // While the window is in error reporting mode no event fires, so an
  // `error` listener that throws is counted and doesn't recurse. A limit
  // isn't an exception to report: it ends the call, as in a timer. Rust asks
  // the engine, which marks the errors it throws for a limit, before any
  // event fires, so no listener sees or cancels one and no script can forge
  // or disguise one. A limit reached while turning `x` into text counts too.
  let reporting = false;
  function reportException(x) {
    if (n.limit(x)) return;
    let message;
    try {
      message = `${x}`;
    } catch (e) {
      if (n.limit(e)) return;
      message = 'exception';
    }
    if (windowState === null || reporting) { report(message); return; }
    const s = make(I_ErrorEvent);
    initEvent(s, 'error', false, true);
    s.isTrusted = true;
    s.message = message; s.filename = ''; s.lineno = 0; s.colno = 0; s.error = x;
    reporting = true;
    let notHandled;
    try {
      notHandled = dispatch(windowState, s);
    } finally {
      reporting = false;
    }
    if (notHandled) report(message);
  }

  // ------ Event handlers (HTML §8.1.7) ------
  //
  // A record's `hm` maps an event handler's name to its state:
  // `value` (a callback, or null), `pending` (`raw` waits to be compiled, in
  // the scope of `scope`), `attr` (the content attribute's text as last
  // seen, or null), `entry` (its listener, or null).
  // Content attributes are read when an event reaches the target and when
  // a listener is added or the handler read, not when they change: all
  // writes to them are in Rust, and a page can't tell the difference, since
  // a listener's place is only observable from those same points.

  // handlerNames[id]: names of interface `id`'s handlers, as a set.
  const handlerNames = create(null);
  // HTML §8.1.7.1, "Window-reflecting body element event handler set": on
  // a body or frameset element these are the window's handlers. The ones
  // only Window has come from the IDL (`handlers`).
  const reflected = create(null);
  for (const name of ['onblur', 'onerror', 'onfocus', 'onload', 'onresize', 'onscroll']) reflected[name] = true;
  function reflects(s, name) {
    if (reflected[name] !== true) return false;
    const local = call(OP_localName, s.g, s.h);
    return local === 'body' || local === 'frameset';
  }

  function handlerState(s, name) {
    if (s.hm === undefined) s.hm = create(null);
    let e = s.hm[name];
    if (e === undefined) {
      e = create(null);
      e.value = null; e.pending = false; e.raw = null; e.scope = null;
      e.attr = null; e.entry = null;
      s.hm[name] = e;
    }
    return e;
  }
  function activate(s, name, e) {
    if (e.entry !== null) return;
    const callback = function (event) { processHandler(s, name, e, event); };
    e.entry = addListener(listenersOf(s, true), apply(slice, name, [2]), callback, false, false, false);
  }
  function deactivate(s, e) {
    e.value = null; e.pending = false;
    if (e.entry === null) return;
    const l = s.l;
    for (let i = 0; i < l.n; i++) if (l[i] === e.entry) { removeAt(l, i); break; }
    e.entry = null;
  }

  // The content attributes of element (g, h) into `s`'s handlers: `s` is
  // the element, or the window for a body element's reflected ones.
  function syncFrom(s, g, h, wanted) {
    const seen = create(null);
    nameList(call(OP_getAttributeNames, g, h), (name) => {
      if (wanted(name) !== true) return;
      seen[name] = true;
      const raw = call(OP_getAttribute, g, h, name);
      const e = handlerState(s, name);
      if (e.attr === raw) return;
      e.attr = raw; e.raw = raw; e.value = null; e.pending = true; e.scope = wrap(g, h);
      activate(s, name, e);
    });
    if (s.hm === undefined) return;
    for (const name in s.hm) {
      const e = s.hm[name];
      if (e.attr !== null && seen[name] !== true) { e.attr = null; deactivate(s, e); }
    }
  }
  function syncHandlers(s) {
    if (s === windowState) {
      const g = current();
      if (g === 0) return;
      const body = call(OP_body, g, root());
      if (body === 0 || body === null) return;
      syncFrom(s, g, body, (name) => reflected[name]);
    } else if (is(s, I_HTMLElement)) {
      const known = handlerNames[I_HTMLElement];
      syncFrom(s, s.g, s.h, (name) => known[name] === true && !reflects(s, name));
    }
  }

  // HTML §8.1.7.1 "getting the current value of the event handler": a
  // pending attribute compiles once. A syntax error is reported and leaves
  // null. `with` gives the handler its scope: the element, then the document
  // (no form owner yet); neither can name a variable of this wrapper, since
  // `arguments` is all it has. The body is parsed alone first, so most text
  // that would close the wrapper is a syntax error; QuickJS's constructor
  // pastes source together, so `}, function () {` isn't, which gets a page
  // nothing it hasn't got in a script.
  function compile(s, name, e) {
    const params = s === windowState && name === 'onerror' ? 'event,source,lineno,colno,error' : 'event';
    try {
      new FunctionCtor(params, e.raw);
      const build = new FunctionCtor(
        `with (arguments[0]) with (arguments[1]) return function ${name}(${params}) {\n${e.raw}\n};`);
      return apply(build, undefined, [wrap(current(), root()), e.scope]);
    } catch (x) {
      reportException(x);
      return null;
    }
  }
  function handlerValue(s, name) {
    syncHandlers(s);
    if (s.hm === undefined) return null;
    const e = s.hm[name];
    if (e === undefined) return null;
    if (e.pending) { e.pending = false; e.value = compile(s, name, e); e.scope = null; }
    return e.value;
  }

  // HTML §8.1.7.1 "the event handler processing algorithm". What it throws
  // goes to `invoke`, like any listener's.
  function processHandler(s, name, e, event) {
    const callback = handlerValue(s, name);
    // [LegacyTreatNonObjectAsNull]: an object that can't be called is kept
    // and does nothing.
    if (typeof callback !== 'function') return;
    const v = stateOf(event);
    const special = s === windowState && v.type === 'error' && is(v, I_ErrorEvent);
    const result = special
      ? apply(callback, s.w, [v.message, v.filename, v.lineno, v.colno, v.error])
      : apply(callback, s.w, [event]);
    if (result === special) v.canceled = true;
  }

  // The event handler IDL attributes of interface `id`.
  function handlers(id, list_) {
    const known = create(null);
    handlerNames[id] = known;
    for (let i = 0; i < list_.length; i++) {
      const name = list_[i];
      known[name] = true;
      // The body element's are the window's.
      const owner = (s) => (id === I_HTMLElement && reflects(s, name) ? windowState : s);
      attr(id, name, function () {
        return handlerValue(owner(self(this, id)), name);
      }, function (v) {
        const s = owner(self(this, id));
        syncHandlers(s);
        if (!isObject(v)) { deactivate(s, handlerState(s, name)); return; }
        const e = handlerState(s, name);
        e.value = v; e.pending = false; e.scope = null;
        activate(s, name, e);
      });
    }
  }
