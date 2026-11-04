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

  function flatten(options) {
    if (typeof options === 'object' && options !== null) return !!options.capture;
    return !!options;
  }
  function listenersOf(s, create_) {
    if (s.l === undefined && create_) s.l = list();
    return s.l;
  }
  H.EventTarget_addEventListener = (s, type, callback, options) => {
    // §2.7.3 "flatten more". An aborted signal adds nothing.
    const capture = flatten(options);
    let once = false, passive = false;
    if (typeof options === 'object' && options !== null) {
      once = !!options.once;
      passive = !!options.passive;
    }
    if (callback === null || callback === undefined) return;
    if (typeof callback !== 'object' && typeof callback !== 'function') {
      throw new TE('addEventListener: the listener is not an object');
    }
    const l = listenersOf(s, true);
    for (let i = 0; i < l.n; i++) {
      const e = l[i];
      if (e.type === type && e.callback === callback && e.capture === capture) return;
    }
    const e = create(null);
    e.type = type; e.callback = callback; e.capture = capture;
    e.once = once; e.passive = passive; e.removed = false;
    add(l, e);
  };
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
    if ((s.m & nodeBit) === 0) return null;
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

  function initEvent(s, type, bubbles, cancelable) {
    s.type = type; s.bubbles = bubbles; s.cancelable = cancelable; s.composed = false;
    s.initialized = true; s.dispatching = false; s.stop = false; s.stopNow = false;
    s.canceled = false; s.passive = false; s.isTrusted = false;
    s.target = null; s.currentTarget = null; s.eventPhase = 0;
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
    if (!s.dispatching) initEvent(s, type, bubbles, cancelable);
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
    initEvent(s, type, bubbles, cancelable);
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
  function reportException(x) {
    let message;
    try { message = `${x}`; } catch (e) { message = 'exception'; }
    if (windowState === null) { report(message); return; }
    const s = make(I_ErrorEvent);
    initEvent(s, 'error', false, true);
    s.isTrusted = true;
    s.message = message; s.filename = ''; s.lineno = 0; s.colno = 0; s.error = x;
    if (dispatch(windowState, s)) report(message);
  }
