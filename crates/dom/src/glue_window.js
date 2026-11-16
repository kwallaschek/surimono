  // Globals a page meets early (P2-DOM-20, part one): matchMedia,
  // requestIdleCallback, the CSS namespace and crypto. Part of the function
  // glue_head.js opens. Interfaces come from the IDL (`JS_INTERFACES` and
  // `DOM_NAMESPACES` in the generator); the bodies are here.

  // ------ matchMedia (CSSOM View §4.2) ------
  //
  // Parsed, serialized and evaluated in Rust (media.rs), once, when the list is
  // made: nothing changes during a render, so `change` never fires and
  // listeners only wait. Evaluated for `screen`, which is what scripts run
  // under; print media is the printer's business. `width` and `height` are the
  // window's, as `innerWidth` and `innerHeight` give them, and a query that
  // reads them is a layout query, asked when `matches` is read.
  const MAX_QUERY = 65536;
  H.Window_matchMedia = (s, query) => {
    const l = make(I_MediaQueryList);
    if (query.length > MAX_QUERY) {
      l.media = 'not all'; l.matched = false; l.sized = false;
      return l.w;
    }
    const r = n.matchMedia(query, windowMetrics.innerWidth, windowMetrics.innerHeight, 1);
    l.matched = apply(charCodeAt, r, [0]) === 49;
    l.sized = apply(charCodeAt, r, [1]) === 49;
    l.media = apply(slice, r, [2]);
    return l.w;
  };
  H.MediaQueryList_media = (s) => s.media;
  H.MediaQueryList_matches = (s) => {
    if (s.sized) askDocument('matchMedia');
    return s.matched;
  };
  // §4.2 "add a listener" and "remove a listener": `change` listeners with no
  // capture, null doing nothing.
  H.MediaQueryList_addListener = (s, callback) => {
    if (callback === null) return;
    H.EventTarget_addEventListener(s, 'change', callback, false);
  };
  H.MediaQueryList_removeListener = (s, callback) => {
    if (callback === null) return;
    H.EventTarget_removeEventListener(s, 'change', callback, false);
  };

  // ------ requestIdleCallback ------
  //
  // The idle periods are the event loop's (crates/core/src/event_loop.rs): a
  // callback runs once nothing else is due, with a deadline at most 50 ms on.
  // The clock stands still in a task, so `timeRemaining()` is the same all
  // through one callback.
  H.Window_requestIdleCallback = (s, callback, options) => {
    if (typeof callback !== 'function') {
      throw new TE("requestIdleCallback: parameter 1 is not of type 'Function'");
    }
    const o = create(null);
    if (options !== undefined && options !== null) {
      if (!isObject(options)) {
        throw new TE("requestIdleCallback: the value is not of type 'IdleRequestOptions'");
      }
      const t = options.timeout;
      if (t !== undefined) o.timeout = t >>> 0;
    }
    return requestIdleReal(function (d) {
      const r = make(I_IdleDeadline);
      r.end = d.deadline; r.timedOut = d.didTimeout;
      // A task, as a timer's: a rendering opportunity first, then WebIDL's
      // "invoke a callback function", which reports what it throws.
      try {
        if (!pageLoading) flushAutofocus();
        apply(callback, G, [r.w]);
      } catch (x) {
        if (!n.limit(x)) n.checkpoint();
        reportException(x, true);
      }
    }, o);
  };
  H.Window_cancelIdleCallback = (s, handle) => { cancelIdleReal(handle); };
  H.IdleDeadline_timeRemaining = (s) => {
    if (s.timedOut) return 0;
    const left = s.end - now();
    return left > 0 ? left : 0;
  };
  H.IdleDeadline_didTimeout = (s) => s.timedOut;

  // ------ CSS (CSSOM §6.1, CSS Conditional 3 §8.1) ------
  //
  // `escape` is CSSOM's "serialize an identifier", on UTF-16 code units, so a
  // lone surrogate stays one. `supports` asks stylo (css_supports.rs).
  const fromCode = String.fromCharCode, imul = Math.imul;
  const toHex = Number.prototype.toString;
  H.CSS_escape = (ident) => {
    // Built a chunk at a time: one `+=` per character of a long string
    // costs the engine far more than the string.
    let out = '', chunk = '';
    const len = ident.length;
    const first = len > 0 ? apply(charCodeAt, ident, [0]) : 0;
    for (let i = 0; i < len; i++) {
      const c = apply(charCodeAt, ident, [i]);
      if (c === 0) chunk += '\uFFFD';
      else if ((c >= 1 && c <= 0x1f) || c === 0x7f ||
        (c >= 0x30 && c <= 0x39 && (i === 0 || (i === 1 && first === 0x2d)))) {
        chunk += '\\' + apply(toHex, c, [16]) + ' ';
      } else if (i === 0 && len === 1 && c === 0x2d) chunk += '\\-';
      else if (c >= 0x80 || c === 0x2d || c === 0x5f || (c >= 0x30 && c <= 0x39) ||
        (c >= 0x41 && c <= 0x5a) || (c >= 0x61 && c <= 0x7a)) {
        chunk += fromCode(c);
      } else chunk += '\\' + fromCode(c);
      if (chunk.length >= 4096) { out += chunk; chunk = ''; }
    }
    return out + chunk;
  };
  H.CSS_supports = (a, b) => n.cssSupports(apply(toWellFormed, a, []),
    b === undefined ? undefined : apply(toWellFormed, b, []));
  function namespace(name) {
    // WebIDL §3.13.1: an ordinary object, Object.prototype above it, with its
    // class string, on the global as an interface object is. Called from
    // generated/bindings.js, before this file's constants exist, and before
    // any page script.
    const o = {};
    defineProperty(o, symbolTag, { value: name, configurable: true });
    defineProperty(G, name, { value: o, writable: true, enumerable: false, configurable: true });
    return o;
  }
  function nsOp(o, name, f) {
    defineProperty(o, name, { value: f, writable: true, enumerable: true, configurable: true });
  }

  // ------ QuotaExceededError (WebIDL §2.8.1.1) ------
  //
  // A DOMException whose name is its own, with what was asked and what was
  // allowed, each null when not given. Chromium 153 has it; WebCrypto throws it.
  class QuotaExceededError extends DOMExceptionCtor {
    #quota; #requested;
    constructor(message = undefined, options = undefined) {
      const m = message === undefined ? '' : `${message}`;
      let quota = null, requested = null;
      if (options !== undefined && options !== null) {
        if (!isObject(options)) throw new TE('QuotaExceededError: options are not a dictionary');
        // WebIDL §3.2.18: members in lexicographic order.
        const q = options.quota;
        if (q !== undefined) quota = dbl(q);
        const r = options.requested;
        if (r !== undefined) requested = dbl(r);
      }
      if (quota !== null && quota < 0) throw new RangeErrorCtor('QuotaExceededError: quota is negative');
      if (requested !== null && requested < 0) {
        throw new RangeErrorCtor('QuotaExceededError: requested is negative');
      }
      if (quota !== null && requested !== null && requested < quota) {
        throw new RangeErrorCtor('QuotaExceededError: requested is less than quota');
      }
      super(m, 'QuotaExceededError');
      this.#quota = quota;
      this.#requested = requested;
    }
    static quotaOf(e) { return e.#quota; }
    static requestedOf(e) { return e.#requested; }
  }
  {
    const P = QuotaExceededError.prototype;
    const quotaOf = QuotaExceededError.quotaOf, requestedOf = QuotaExceededError.requestedOf;
    delete QuotaExceededError.quotaOf;
    delete QuotaExceededError.requestedOf;
    defineProperty(P, 'quota', { get: rdescriptor({ get quota() { return quotaOf(this); } }, 'quota').get, enumerable: true, configurable: true });
    defineProperty(P, 'requested', { get: rdescriptor({ get requested() { return requestedOf(this); } }, 'requested').get, enumerable: true, configurable: true });
    defineProperty(P, symbolTag, { value: 'QuotaExceededError', configurable: true });
    defineProperty(G, 'QuotaExceededError', { value: QuotaExceededError, writable: true, enumerable: false, configurable: true });
  }

  // ------ crypto (WebCrypto §10) ------
  //
  // From the render's seed, the one Math.random has, through a stream of its
  // own (xoshiro128**, seeded through splitmix32 with the halves turned), so
  // drawing here leaves Math.random's numbers as they were. Not secret and not
  // random across renders, by design: same input, same output.
  function splitmix(a) {
    return function () {
      a = (a + 0x9e3779b9) | 0;
      let t = a ^ (a >>> 16);
      t = imul(t, 0x21f0aaad);
      t = t ^ (t >>> 15);
      t = imul(t, 0x735a2d97);
      return (t ^ (t >>> 15)) | 0;
    };
  }
  const seedA = splitmix((n.seedHi ^ 0x63727970) | 0), seedB = splitmix((n.seedLo ^ 0x746f2121) | 0);
  let r0 = seedA(), r1 = seedB(), r2 = seedA(), r3 = seedB();
  if ((r0 | r1 | r2 | r3) === 0) r0 = 1;
  function rotl(x, k) { return (x << k) | (x >>> (32 - k)); }
  function next32() {
    const r = imul(rotl(imul(r1, 5), 7), 9);
    const t = r1 << 9;
    r2 ^= r0; r3 ^= r1; r1 ^= r2; r0 ^= r3; r2 ^= t; r3 = rotl(r3, 11);
    return r >>> 0;
  }
  function fill(bytes, len) {
    let i = 0;
    for (; i + 4 <= len; i += 4) {
      const x = next32();
      bytes[i] = x & 255; bytes[i + 1] = (x >>> 8) & 255;
      bytes[i + 2] = (x >>> 16) & 255; bytes[i + 3] = x >>> 24;
    }
    if (i < len) {
      let x = next32();
      for (; i < len; i++) { bytes[i] = x & 255; x >>>= 8; }
    }
  }
  // The view and buffer accessors are glue_platform.js's.
  const resizableOf = rdescriptor(ArrayBuffer.prototype, 'resizable');
  const bufferResizable = resizableOf === undefined ? undefined : resizableOf.get;
  // WebIDL §3.2.25: an ArrayBufferView without [AllowShared] or
  // [AllowResizable] is over a fixed-length ArrayBuffer.
  function plainBuffer(buffer) {
    try {
      apply(abLength, buffer, []);
    } catch (e) {
      throw new TE("getRandomValues: the ArrayBufferView must not be shared");
    }
    if (bufferResizable !== undefined && apply(bufferResizable, buffer, [])) {
      throw new TE("getRandomValues: the ArrayBufferView must not be resizable");
    }
  }
  const integerArrays = create(null);
  for (const t of ['Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array', 'Uint16Array',
    'Int32Array', 'Uint32Array', 'BigInt64Array', 'BigUint64Array']) integerArrays[t] = true;
  // §10.1.1, with the type checks in Chromium's order: the view's type, then
  // the quota.
  H.Crypto_getRandomValues = (s, view) => {
    const tag = isObject(view) ? apply(taTag, view, []) : undefined;
    if (tag === undefined) {
      let buffer;
      try {
        buffer = apply(dvBuffer, view, []);
      } catch (e) {
        throw new TE("getRandomValues: parameter 1 is not of type 'ArrayBufferView'");
      }
      plainBuffer(buffer);
      throwDom('TypeMismatchError', "getRandomValues: the ArrayBufferView is a DataView, not an integer array");
    }
    const buffer = apply(taBuffer, view, []);
    plainBuffer(buffer);
    if (integerArrays[tag] !== true) {
      throwDom('TypeMismatchError', `getRandomValues: the ArrayBufferView is a ${tag}, not an integer array`);
    }
    const len = apply(taByteLength, view, []);
    if (len > 65536) {
      throw new QuotaExceededError(`getRandomValues: ${len} bytes is more than the 65536 one call may have`);
    }
    // A detached buffer's view has length 0, and nothing to fill.
    if (len > 0) fill(new U8(buffer, apply(taOffset, view, []), len), len);
    return view;
  };
  // §10.1.2, version 4, from the same stream.
  const uuidBytes = new U8(16);
  const hexDigits = '0123456789abcdef';
  H.Crypto_randomUUID = () => {
    fill(uuidBytes, 16);
    uuidBytes[6] = (uuidBytes[6] & 0x0f) | 0x40;
    uuidBytes[8] = (uuidBytes[8] & 0x3f) | 0x80;
    let out = '';
    for (let i = 0; i < 16; i++) {
      if (i === 4 || i === 6 || i === 8 || i === 10) out += '-';
      const b = uuidBytes[i];
      out += hexDigits[b >>> 4] + hexDigits[b & 15];
    }
    return out;
  };
  // Secure Contexts §3.1: the page's URL decides, once, as the realm is made
  // (web.rs). `randomUUID` is [SecureContext]: elsewhere it isn't there.
  const secureContext = n.secureContext();
  if (!secureContext) delete protos[I_Crypto].randomUUID;
  H.Window_isSecureContext = () => secureContext;
  const cryptoState = make(I_Crypto);
  H.Window_crypto = () => cryptoState.w;
