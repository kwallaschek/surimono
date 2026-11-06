
  // ------ Location and History (HTML §7.2.4, §7.2.5) ------
  //
  // A printed page goes nowhere, so nothing here leaves the document: what
  // would navigate to another one stops there, as a link and a form's submit
  // do (glue_activation.js), and `reload` and `go(0)` do nothing. What stays in
  // the document happens: a fragment, a `javascript:` URL, `pushState` and
  // `replaceState`, and traversal among the entries they made. Rust has the URL
  // and the parser; a document's two objects hang off its record.

  const MapCtor = Map, SetCtor = Set, DateCtor = Date, RegExpCtor = RegExp, ArrayCtor = Array;
  const mapGet = Map.prototype.get, mapSet = Map.prototype.set;
  const mapForEach = Map.prototype.forEach, setForEach = Set.prototype.forEach;
  const mapHas = Map.prototype.has, setHas = Set.prototype.has, setAdd = Set.prototype.add;
  const dateTime = Date.prototype.getTime;
  const regexpSource = rdescriptor(RegExp.prototype, 'source').get;
  const regexpFlags = rdescriptor(RegExp.prototype, 'flags').get;
  const isArray = Array.isArray, keysOf = Object.keys;

  function locationOf(d) {
    if (d.location === undefined) {
      const l = make(I_Location);
      l.doc = d;
      d.location = l.w;
    }
    return d.location;
  }
  const pageState = () => (current() === 0 ? null : documentState());
  H.Window_location = () => {
    const d = pageState();
    return d === null ? null : locationOf(d);
  };
  const urlPart = (name) => (s) => call(OP_urlPart, s.doc.g, s.doc.h, name);
  H.Location_href = urlPart('href');
  H.Location_origin = urlPart('origin');
  H.Location_protocol = urlPart('protocol');
  H.Location_host = urlPart('host');
  H.Location_hostname = urlPart('hostname');
  H.Location_port = urlPart('port');
  H.Location_pathname = urlPart('pathname');
  H.Location_search = urlPart('search');
  H.Location_hash = urlPart('hash');
  // What these do is navigate, which stops here.
  const staysHere = () => {};
  H.set_Location_protocol = staysHere;
  H.set_Location_host = staysHere;
  H.set_Location_hostname = staysHere;
  H.set_Location_port = staysHere;
  H.set_Location_pathname = staysHere;
  H.set_Location_search = staysHere;
  H.set_Location_href = (s, url) => { navigate(s.doc, url, false, true); };
  H.Location_assign = (s, url) => { navigate(s.doc, url, false, true); };
  H.Location_replace = (s, url) => { navigate(s.doc, url, true, true); };
  H.Location_reload = staysHere;
  // HTML §7.2.4: Rust answers empty for a fragment that is already the URL's.
  H.set_Location_hash = (s, v) => {
    const url = call(OP_urlFragment, s.doc.g, s.doc.h, `${v}`);
    if (url !== '') followFragment(s.doc, url, pageLoading && isPage(s.doc));
  };

  const noFragment = (url) => {
    const i = apply(indexOf, url, ['#']);
    return i < 0 ? url : apply(slice, url, [0, i]);
  };

  // The page's lifecycle is running: until its `load` has fired, a Location
  // navigation replaces (HTML §7.2.4 "Location-object navigate").
  let pageLoading = false;

  // HTML §7.4.2 "navigate", for the URLs that don't leave the page. The URL
  // is parsed first, so its scheme is what the URL Standard says it is.
  function navigate(d, url, replace, fromScript) {
    const target = call(OP_urlJoin, d.g, d.h, url);
    if (target === '') {
      if (fromScript) throwDom('SyntaxError', 'the URL can\'t be parsed');
      return;
    }
    if (apply(slice, target, [0, 11]) === 'javascript:') {
      if (!isPage(d)) return;
      const source = percentDecode(apply(slice, target, [11]));
      task(() => {
        try {
          n.runSource(apply(toWellFormed, source, []));
        } catch (x) {
          reportException(x);
        }
      }, 0);
      return;
    }
    if (apply(indexOf, target, ['#']) >= 0 &&
        noFragment(target) === noFragment(call(OP_urlPart, d.g, d.h, 'href'))) {
      followFragment(d, target, replace || (fromScript && pageLoading && isPage(d)));
    }
  }

  // HTML §7.4.2.3.3 "navigate to a fragment" and "update document for history
  // step application": the same document, a new entry with no state, `popstate`
  // now and `hashchange` in a task, both at the page only. To the URL it already
  // has, the navigation replaces (§7.4.2), and no fragment changed. The record
  // comes first: it reads the URL for its first entry. Rust then takes the URL,
  // so a refusal leaves the entries alone.
  function followFragment(d, url, replace) {
    const old = call(OP_urlPart, d.g, d.h, 'href');
    const same = url === old;
    const h = historyRecord(d);
    call(OP_setDocumentUrl, d.g, d.h, url);
    if (replace || same) h.e[h.i] = entry(url, null);
    else push(h, url, null);
    h.state = null;
    if (!isPage(d)) return;
    firePopState(null, false);
    if (!same) task(() => { fireHashChange(old, url); }, 0);
  }

  function firePopState(state, alone) {
    const e = make(I_PopStateEvent);
    initEvent(e, 'popstate', false, false);
    e.isTrusted = true;
    e.alone = alone;
    e.d = eventInits[I_PopStateEvent]();
    e.d.state = state;
    dispatch(windowState, e);
  }

  // Always from a task of its own: see `fireInTask`.
  function fireHashChange(old, url) {
    const e = make(I_HashChangeEvent);
    initEvent(e, 'hashchange', false, false);
    e.isTrusted = true;
    e.alone = true;
    e.d = eventInits[I_HashChangeEvent]();
    e.d.oldURL = old; e.d.newURL = url;
    dispatch(windowState, e);
  }

  function entry(url, state) {
    const x = create(null);
    x.url = url; x.state = state;
    return x;
  }

  // The session history entries of one document: all there is, no document
  // being left. `n` entries in `e`, `i` the current one.
  function historyRecord(d) {
    if (d.history === undefined) {
      const h = make(I_History);
      h.doc = d; h.e = create(null); h.n = 1; h.i = 0; h.state = null; h.restoration = 'auto';
      h.e[0] = entry(call(OP_urlPart, d.g, d.h, 'href'), null);
      d.history = h;
    }
    return d.history;
  }
  // The entries after the current one go, as the standard says of a new one.
  function push(h, url, state) {
    for (let k = h.i + 1; k < h.n; k++) delete h.e[k];
    h.i += 1;
    h.n = h.i + 1;
    h.e[h.i] = entry(url, state);
  }
  H.Window_history = () => {
    const d = pageState();
    return d === null ? null : historyRecord(d).w;
  };
  H.History_length = (h) => h.n;
  H.History_state = (h) => h.state;
  H.History_scrollRestoration = (h) => h.restoration;
  H.set_History_scrollRestoration = (h, v) => {
    const x = `${v}`;
    if (x === 'auto' || x === 'manual') h.restoration = x;
  };

  function dataClone() {
    throwDom('DataCloneError', 'the value can\'t be cloned');
  }
  // Whether `method` of a built-in accepts `v`: its brand, which neither a
  // prototype nor Symbol.toStringTag can fake.
  function branded(method, v) {
    try {
      apply(method, v, []);
      return true;
    } catch (x) {
      if (n.limit(x)) throw x;
      return false;
    }
  }
  function own(out, k, v) {
    const d = create(null);
    d.value = v; d.writable = true; d.enumerable = true; d.configurable = true;
    rdefine(out, k, d);
  }
  const objectProto = Object.prototype, objectToString = objectProto.toString;
  const hasOwn = Object.prototype.hasOwnProperty, getProto = Object.getPrototypeOf;
  const abProto = ArrayBuffer.prototype, BufferCtor = ArrayBuffer, Bytes = Uint8Array;
  const abBytes = rdescriptor(abProto, 'byteLength').get;
  const abDetached = rdescriptor(abProto, 'detached').get;
  const sharedBytes = rdescriptor(SharedArrayBuffer.prototype, 'byteLength').get;
  const abTransfer = abProto.transfer;
  const bytesSet = Object.getPrototypeOf(Uint8Array.prototype).set;
  const typedTag = rdescriptor(Object.getPrototypeOf(Uint8Array.prototype), Symbol.toStringTag).get;
  const typedProps = Object.getPrototypeOf(Uint8Array.prototype);
  const typedBuffer = rdescriptor(typedProps, 'buffer').get;
  const typedOffset = rdescriptor(typedProps, 'byteOffset').get;
  const typedLength = rdescriptor(typedProps, 'length').get;
  const viewProps = DataView.prototype, ViewCtor = DataView;
  const viewBuffer = rdescriptor(viewProps, 'buffer').get;
  const viewOffset = rdescriptor(viewProps, 'byteOffset').get;
  const viewLength = rdescriptor(viewProps, 'byteLength').get;
  const typedCtors = create(null);
  for (const c of [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array,
                   Uint32Array, Float16Array, Float32Array, Float64Array, BigInt64Array,
                   BigUint64Array]) {
    typedCtors[c.name] = c;
  }
  const boxes = [Boolean, Number, String, BigInt];
  const boxValueOf = [Boolean.prototype.valueOf, Number.prototype.valueOf,
                      String.prototype.valueOf, BigInt.prototype.valueOf];
  const ObjectCtor = Object, symbolValueOf = Symbol.prototype.valueOf;
  const weakHas = [WeakMap.prototype.has, WeakSet.prototype.has];
  const errorCtors = create(null);
  for (const c of [Error, EvalError, RangeError, ReferenceError, SyntaxError, TypeError, URIError]) {
    errorCtors[c.name] = c;
  }
  const isDetached = (b) => apply(abDetached, b, []);
  const domExceptionName = rdescriptor(DOMExceptionCtor.prototype, 'name').get;
  const domExceptionMessage = rdescriptor(DOMExceptionCtor.prototype, 'message').get;
  function cloneBuffer(b) {
    const out = new BufferCtor(apply(abBytes, b, []));
    apply(bytesSet, new Bytes(out), [new Bytes(b)]);
    return out;
  }
  // What only a plain object can be spared: the brand tests each cost a thrown
  // exception, and a plain object is what a clone is mostly made of. A
  // Symbol.toStringTag anywhere up the chain goes the long way, as anything
  // built in that hides its class does.
  const plain = (v) => getProto(v) === objectProto && !rhas(v, symbolTag) &&
    apply(objectToString, v, []) === '[object Object]';
  // HTML §2.7.3 StructuredSerializeInternal and its deserialization in one step.
  // Recursion ends at the sandbox's stack limit, and the copy is under its
  // memory limit.
  function cloneValue(v, seen) {
    const t = typeof v;
    if (t === 'symbol' || t === 'function') dataClone();
    if (t !== 'object' || v === null) return v;
    if (stateOf(v) !== null) dataClone();
    const known = apply(mapGet, seen, [v]);
    if (known !== undefined) return known;
    const kind = n.valueKind(v);
    if (kind === 1 || kind === 3) dataClone();
    if (kind === 2) return cloneError(v, seen);
    let out;
    if (isArray(v)) {
      out = new ArrayCtor(v.length);
    } else if (plain(v)) {
      out = {};
    } else {
      out = cloneBuiltIn(v, seen);
      if (out !== undefined) {
        apply(mapSet, seen, [v, out]);
        return out;
      }
      out = {};
    }
    apply(mapSet, seen, [v, out]);
    const keys = keysOf(v);
    const m = keys.length;
    for (let i = 0; i < m; i++) {
      const k = keys[i];
      if (apply(hasOwn, v, [k])) own(out, k, cloneValue(v[k], seen));
    }
    return out;
  }
  // An object with a class of its own, or undefined for one that clones as a
  // plain object. Map and Set are filled here, once `seen` has them.
  function cloneBuiltIn(v, seen) {
    if (branded(dateTime, v)) return new DateCtor(apply(dateTime, v, []));
    if (v !== RegExpCtor.prototype && branded(regexpSource, v)) {
      return new RegExpCtor(apply(regexpSource, v, []), apply(regexpFlags, v, []));
    }
    if (branded(mapHas, v) || branded(setHas, v)) return cloneCollection(v, seen);
    if (branded(domExceptionName, v)) {
      return new DOMExceptionCtor(apply(domExceptionMessage, v, []), apply(domExceptionName, v, []));
    }
    if (branded(sharedBytes, v)) dataClone();
    if (branded(abBytes, v)) {
      if (isDetached(v)) dataClone();
      return cloneBuffer(v);
    }
    const tag = apply(typedTag, v, []);
    if (tag !== undefined) {
      const buffer = cloneValue(apply(typedBuffer, v, []), seen);
      return new typedCtors[tag](buffer, apply(typedOffset, v, []), apply(typedLength, v, []));
    }
    if (branded(viewLength, v)) {
      const buffer = cloneValue(apply(viewBuffer, v, []), seen);
      return new ViewCtor(buffer, apply(viewOffset, v, []), apply(viewLength, v, []));
    }
    for (let i = 0; i < 4; i++) {
      if (branded(boxValueOf[i], v)) return ObjectCtor(apply(boxValueOf[i], v, []));
    }
    if (branded(symbolValueOf, v) || branded(weakHas[0], v) || branded(weakHas[1], v)) dataClone();
    return undefined;
  }
  // §2.7.3: the name from the standard's list, own data `message` and `cause`. Chrome
  // keeps `stack` too, which the standard allows.
  function cloneError(v, seen) {
    const name = v.name;
    const ctor = (typeof name === 'string' ? errorCtors[name] : undefined) ?? errorCtors.Error;
    const m = rdescriptor(v, 'message');
    const message = m !== undefined && 'value' in m ? `${m.value}` : undefined;
    const out = message === undefined ? new ctor() : new ctor(message);
    const stack = rdescriptor(v, 'stack');
    if (stack !== undefined && typeof stack.value === 'string') own(out, 'stack', stack.value);
    apply(mapSet, seen, [v, out]);
    const cause = rdescriptor(v, 'cause');
    if (cause !== undefined && 'value' in cause) own(out, 'cause', cloneValue(cause.value, seen));
    return out;
  }
  function cloneCollection(v, seen) {
    const isMap = branded(mapHas, v);
    const out = isMap ? new MapCtor() : new SetCtor();
    apply(mapSet, seen, [v, out]);
    if (isMap) {
      apply(mapForEach, v, [(x, k) => { apply(mapSet, out, [cloneValue(k, seen), cloneValue(x, seen)]); }]);
    } else {
      apply(setForEach, v, [(x) => { apply(setAdd, out, [cloneValue(x, seen)]); }]);
    }
    return out;
  }

  // HTML §7.2.5 "shared history push/replace state steps".
  function pushOrReplace(h, data, url, replace) {
    const d = h.doc;
    const state = cloneValue(data, new MapCtor());
    let target = call(OP_urlPart, d.g, d.h, 'href');
    if (url !== null && url !== '') {
      target = call(OP_urlRewrite, d.g, d.h, url);
      if (target === '') {
        throwDom('SecurityError', 'the document\'s URL can\'t be rewritten to that URL');
      }
    }
    // Rust takes the URL first: a refusal leaves the entries alone.
    call(OP_setDocumentUrl, d.g, d.h, target);
    if (replace) h.e[h.i] = entry(target, state);
    else push(h, target, state);
    h.state = state;
  }
  H.History_pushState = (h, data, unused, url) => { pushOrReplace(h, data, url, false); };
  H.History_replaceState = (h, data, unused, url) => { pushOrReplace(h, data, url, true); };

  // HTML §7.2.5 "traverse the history by a delta", among this document's
  // entries: in a task, as the standard's are.
  function traverse(h, delta) {
    if (delta === 0) return;
    task(() => {
      const at = h.i + delta;
      if (at < 0 || at >= h.n) return;
      const d = h.doc;
      const old = call(OP_urlPart, d.g, d.h, 'href');
      const x = h.e[at];
      h.i = at;
      h.state = x.state;
      call(OP_setDocumentUrl, d.g, d.h, x.url);
      if (!isPage(d)) return;
      firePopState(x.state, true);
      if (x.url !== old && noFragment(x.url) === noFragment(old)) fireHashChange(old, x.url);
    }, 0);
  }
  H.History_go = (h, delta) => { traverse(h, delta); };
  H.History_back = (h) => { traverse(h, -1); };
  H.History_forward = (h) => { traverse(h, 1); };
