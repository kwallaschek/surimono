  // ------ What the window has that no node does ------
  //
  // URL, URLSearchParams (URL Standard), TextEncoder, TextDecoder (Encoding
  // Standard), atob, btoa, structuredClone, navigator and customElements. All of
  // it stays in JavaScript values, under the memory limit: a URL is its
  // serialization in the record, and Rust (src/web.rs) answers from that string.

  const getPrototypeOf = getProto, freeze = Object.freeze;
  const U8 = Uint8Array, RangeErrorCtor = RangeError;
  const typedProto = getPrototypeOf(U8.prototype);
  const taBuffer = rdescriptor(typedProto, 'buffer').get;
  const taOffset = rdescriptor(typedProto, 'byteOffset').get;
  const taByteLength = rdescriptor(typedProto, 'byteLength').get;
  const taLength = rdescriptor(typedProto, 'length').get;
  const taTag = rdescriptor(typedProto, symbolTag).get;
  const dvBuffer = rdescriptor(DataView.prototype, 'buffer').get;
  const dvOffset = rdescriptor(DataView.prototype, 'byteOffset').get;
  const dvLength = rdescriptor(DataView.prototype, 'byteLength').get;
  const abLength = rdescriptor(ArrayBuffer.prototype, 'byteLength').get;
  const sabLength = rdescriptor(SharedArrayBuffer.prototype, 'byteLength').get;
  const isView = ArrayBuffer.isView;
  const NAME_START = 97, NAME_END = 122;
  const promiseReject = Promise.reject;
  const toPromiseRejected = (x) => apply(promiseReject, PromiseCtor, [x]);

  // ------ URL (URL Standard §6.1) ------

  function invalidUrl() { throw new TE('Invalid URL'); }
  function newUrl(target, href) {
    const s = make(I_URL, target);
    s.href = href;
    s.params = null;
    return s.w;
  }
  H.new_URL = (target, url, base) => {
    const href = n.urlParse(url, base);
    if (href === undefined) invalidUrl();
    return newUrl(target, href);
  };
  H.URL_parse = (url, base) => {
    const href = n.urlParse(url, base);
    return href === undefined ? null : newUrl(undefined, href);
  };
  H.URL_canParse = (url, base) => n.urlParse(url, base) !== undefined;
  H.URL_href = (s) => s.href;
  H.URL_toJSON = (s) => s.href;
  function urlAssign(s, href) {
    if (href === undefined) throw new TE('the URL is too long');
    s.href = href;
  }
  function refillParams(s) {
    if (s.params !== null) s.params.l = parseQuery(n.urlGet(s.href, 'search'));
  }
  H.set_URL_href = (s, v) => {
    const href = n.urlParse(v, undefined);
    if (href === undefined) invalidUrl();
    s.href = href;
    refillParams(s);
  };
  const urlPart_ = (part) => (s) => n.urlGet(s.href, part);
  const urlWrite = (part) => (s, v) => { urlAssign(s, n.urlSet(s.href, part, v)); };
  H.URL_origin = urlPart_('origin');
  H.URL_protocol = urlPart_('protocol');
  H.set_URL_protocol = urlWrite('protocol');
  H.URL_username = urlPart_('username');
  H.set_URL_username = urlWrite('username');
  H.URL_password = urlPart_('password');
  H.set_URL_password = urlWrite('password');
  H.URL_host = urlPart_('host');
  H.set_URL_host = urlWrite('host');
  H.URL_hostname = urlPart_('hostname');
  H.set_URL_hostname = urlWrite('hostname');
  H.URL_port = urlPart_('port');
  H.set_URL_port = urlWrite('port');
  H.URL_pathname = urlPart_('pathname');
  H.set_URL_pathname = urlWrite('pathname');
  H.URL_search = urlPart_('search');
  H.URL_hash = urlPart_('hash');
  H.set_URL_hash = urlWrite('hash');
  H.set_URL_search = (s, v) => {
    urlAssign(s, n.urlSet(s.href, 'search', v));
    refillParams(s);
  };
  H.URL_searchParams = (s) => {
    if (s.params === null) {
      const p = make(I_URLSearchParams);
      p.l = parseQuery(n.urlGet(s.href, 'search'));
      p.url = s;
      s.params = p;
    }
    return s.params.w;
  };
  defineProperty(G, 'webkitURL', { value: ctors[I_URL], writable: true, configurable: true });

  // ------ URLSearchParams (URL Standard §6.2) ------
  //
  // `l` is the list of name-value pairs, each a null-prototype record.

  function pair(k, v) {
    const x = create(null);
    x.k = k; x.v = v;
    return x;
  }
  // §5.1 "application/x-www-form-urlencoded parser". What Rust gets is one
  // name or value, so that `=` and `&` have been dealt with.
  function parseQuery(text) {
    const out = list();
    const q = text.length > 0 && text[0] === '?' ? apply(slice, text, [1]) : text;
    let at = 0;
    while (at <= q.length) {
      let end = apply(indexOf, q, ['&', at]);
      if (end < 0) end = q.length;
      if (end > at) {
        // Searched in the piece alone: `a&a&a…` with no `=` stays linear.
        const piece = apply(slice, q, [at, end]);
        const eq = apply(indexOf, piece, ['=']);
        const name = eq < 0 ? piece : apply(slice, piece, [0, eq]);
        const value = eq < 0 ? '' : apply(slice, piece, [eq + 1]);
        add(out, pair(n.formDecode(name), n.formDecode(value)));
      }
      at = end + 1;
    }
    return out;
  }
  // §5.2.
  function serializeQuery(l) {
    let out = '';
    for (let i = 0; i < l.n; i++) {
      if (i > 0) out += '&';
      out += `${n.formEncode(l[i].k)}=${n.formEncode(l[i].v)}`;
    }
    return out;
  }
  // §6.2 "update": the URL's query is the list.
  function updateUrl(p) {
    if (p.url !== null) urlAssign(p.url, n.urlSet(p.url.href, 'search', serializeQuery(p.l)));
  }
  // The init's three forms, as WebIDL §3.2.25 tells them apart.
  H.new_URLSearchParams = (target, init) => {
    const p = make(I_URLSearchParams, target);
    p.url = null;
    p.l = list();
    if (isObject(init)) {
      const method = init[symbolIterator];
      if (method !== undefined && method !== null) {
        if (typeof method !== 'function') throw new TE('URLSearchParams: not iterable');
        const pairs = seq(init, (e) => seq(e, (x) => str(x)));
        const m = pairs.length;
        for (let i = 0; i < m; i++) {
          const e = rdescriptor(pairs, i).value;
          if (e.length !== 2) throw new TE('URLSearchParams: a pair needs two strings');
          add(p.l, pair(rdescriptor(e, 0).value, rdescriptor(e, 1).value));
        }
      } else {
        // record<USVString, USVString>: a later key with one spelling replaces.
        const keys = rownKeys(init);
        const m = keys.length;
        const at = create(null);
        for (let i = 0; i < m; i++) {
          const k = keys[i];
          const d = rdescriptor(init, k);
          if (d === undefined || !d.enumerable) continue;
          if (typeof k === 'symbol') throw new TE('URLSearchParams: a key is a symbol');
          const name = str(k), value = str(init[k]);
          const key = `$${name}`;
          if (at[key] !== undefined) {
            at[key].v = value;
          } else {
            at[key] = pair(name, value);
            add(p.l, at[key]);
          }
        }
      }
    } else if (init !== undefined) {
      p.l = parseQuery(str(init));
    }
    return p.w;
  };
  H.URLSearchParams_size = (p) => p.l.n;
  H.URLSearchParams_append = (p, k, v) => {
    add(p.l, pair(k, v));
    updateUrl(p);
  };
  function dropPairs(p, keep) {
    const out = list();
    for (let i = 0; i < p.l.n; i++) if (keep(p.l[i])) add(out, p.l[i]);
    p.l = out;
  }
  H.URLSearchParams_delete = (p, k, v) => {
    dropPairs(p, (x) => x.k !== k || (v !== undefined && x.v !== v));
    updateUrl(p);
  };
  H.URLSearchParams_get = (p, k) => {
    for (let i = 0; i < p.l.n; i++) if (p.l[i].k === k) return p.l[i].v;
    return null;
  };
  H.URLSearchParams_getAll = (p, k) => {
    const out = list();
    for (let i = 0; i < p.l.n; i++) if (p.l[i].k === k) add(out, p.l[i].v);
    return realArray(out);
  };
  H.URLSearchParams_has = (p, k, v) => {
    for (let i = 0; i < p.l.n; i++) {
      if (p.l[i].k === k && (v === undefined || p.l[i].v === v)) return true;
    }
    return false;
  };
  H.URLSearchParams_set = (p, k, v) => {
    let found = false;
    dropPairs(p, (x) => {
      if (x.k !== k) return true;
      if (found) return false;
      found = true;
      x.v = v;
      return true;
    });
    if (!found) add(p.l, pair(k, v));
    updateUrl(p);
  };
  // §6.2: stable, by code units of the name. A merge sort, so that nothing of
  // Array.prototype is involved.
  H.URLSearchParams_sort = (p) => {
    let from = p.l;
    const m = from.n;
    for (let width = 1; width < m; width *= 2) {
      const to = list();
      for (let lo = 0; lo < m; lo += 2 * width) {
        const mid = lo + width < m ? lo + width : m;
        const hi = lo + 2 * width < m ? lo + 2 * width : m;
        let a = lo, b = mid;
        while (a < mid || b < hi) {
          if (b === hi || (a < mid && from[a].k <= from[b].k)) add(to, from[a++]);
          else add(to, from[b++]);
        }
      }
      from = to;
    }
    p.l = from;
    updateUrl(p);
  };
  H.URLSearchParams_toString = (p) => serializeQuery(p.l);

  // WebIDL §3.7.10 pair iterators, over the live list.
  const iteratorProto = create(getPrototypeOf(getPrototypeOf(arrayProto[symbolIterator].call([]))));
  defineProperty(iteratorProto, symbolTag, { value: 'URLSearchParams Iterator', configurable: true });
  function pairIterator(p, kind) {
    const r = create(null);
    r.m = create(null); r.p = p; r.kind = kind; r.i = 0;
    return new Brand(create(iteratorProto), r);
  }
  defineProperty(iteratorProto, 'next', {
    value: {
      next() {
        const r = stateOf(this);
        if (r === null || r.kind === undefined) throw new TE('Illegal invocation');
        if (r.p === null || r.i >= r.p.l.n) { r.p = null; return { value: undefined, done: true }; }
        const x = r.p.l[r.i++];
        const value = r.kind === 0 ? [x.k, x.v] : r.kind === 1 ? x.k : x.v;
        return { value, done: false };
      },
    }.next,
    writable: true, enumerable: true, configurable: true,
  });
  op(I_URLSearchParams, 'forEach', function forEach(callback) {
    const p = self(this, I_URLSearchParams);
    need(arguments.length, 1, 'forEach');
    if (typeof callback !== 'function') throw new TE('forEach: the callback is not a function');
    const that = arguments[1];
    for (let i = 0; i < p.l.n; i++) {
      const x = p.l[i];
      apply(callback, that, [x.v, x.k, this]);
    }
  });
  op(I_URLSearchParams, 'keys', function keys() { return pairIterator(self(this, I_URLSearchParams), 1); });
  op(I_URLSearchParams, 'values', function values() { return pairIterator(self(this, I_URLSearchParams), 2); });
  const entries = function entries() { return pairIterator(self(this, I_URLSearchParams), 0); };
  op(I_URLSearchParams, 'entries', entries);
  defineProperty(protos[I_URLSearchParams], symbolIterator, { value: entries, writable: true, configurable: true });

  // ------ TextEncoder, TextDecoder (Encoding Standard §7) ------

  H.new_TextEncoder = (target) => make(I_TextEncoder, target).w;
  H.TextEncoder_encoding = () => 'utf-8';
  H.TextEncoder_encode = (s, input) => new U8(n.utf8Encode(input));
  // §7.2: as much of `source` as fits, whole scalar values only. The argument
  // is well-formed by now. Writing to a typed array's index goes to the array,
  // whatever a script did to its prototype.
  H.TextEncoder_encodeInto = (s, source, destination) => {
    if (apply(taTag, destination, []) !== 'Uint8Array') {
      throw new TE('encodeInto: the destination is not a Uint8Array');
    }
    const room = apply(taLength, destination, []);
    const m = source.length;
    let read = 0, written = 0;
    while (read < m) {
      let c = apply(charCodeAt, source, [read]);
      let units = 1;
      if (c >= 0xd800 && c <= 0xdbff) {
        c = 0x10000 + ((c - 0xd800) << 10) + apply(charCodeAt, source, [read + 1]) - 0xdc00;
        units = 2;
      }
      const need_ = c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
      if (written + need_ > room) break;
      if (need_ === 1) {
        destination[written] = c;
      } else {
        const lead = need_ === 2 ? 0xc0 : need_ === 3 ? 0xe0 : 0xf0;
        destination[written] = lead | (c >> (6 * (need_ - 1)));
        for (let i = 1; i < need_; i++) {
          destination[written + i] = 0x80 | ((c >> (6 * (need_ - 1 - i))) & 63);
        }
      }
      written += need_;
      read += units;
    }
    return { read, written };
  };

  H.new_TextDecoder = (target, label, options) => {
    const d = dictionary(options, 'TextDecoderOptions');
    const fatal = !!d.fatal, ignoreBOM = !!d.ignoreBOM;
    const name = n.encodingName(label);
    if (name === '') {
      throw new RangeErrorCtor(`the encoding label ${label} is not supported`);
    }
    const s = make(I_TextDecoder, target);
    s.name = name; s.fatal = fatal; s.ignoreBOM = ignoreBOM; s.slot = 0;
    return s.w;
  };
  H.TextDecoder_encoding = (s) => s.name;
  H.TextDecoder_fatal = (s) => s.fatal;
  H.TextDecoder_ignoreBOM = (s) => s.ignoreBOM;
  // AllowSharedBufferSource's bytes, as a view of ours. A detached buffer is
  // empty, as WebIDL's "get a copy of the bytes" has it.
  function bytesView(x) {
    if (x === undefined) return new U8(0);
    if (isView(x)) {
      const typed = apply(taTag, x, []) !== undefined;
      const buffer = apply(typed ? taBuffer : dvBuffer, x, []);
      if (branded(abLength, buffer) && isDetached(buffer)) return new U8(0);
      if (typed) {
        return new U8(buffer, apply(taOffset, x, []), apply(taByteLength, x, []));
      }
      return new U8(buffer, apply(dvOffset, x, []), apply(dvLength, x, []));
    }
    if (isObject(x) && (branded(abLength, x) || branded(sabLength, x))) {
      return apply(branded(abLength, x) ? abLength : sabLength, x, []) === 0 ? new U8(0) : new U8(x);
    }
    throw new TE('the input is not a buffer');
  }
  // A byte per character, the one way bytes reach Rust with no `unsafe`. Chunks
  // go through a null-prototype list, whose `length` and indexes are its own.
  function latin1(view) {
    const m = apply(taLength, view, []);
    let out = '';
    for (let at = 0; at < m; at += 8192) {
      const end = at + 8192 < m ? at + 8192 : m;
      const chunk = create(null);
      chunk.length = end - at;
      for (let i = at; i < end; i++) chunk[i - at] = view[i];
      out += apply(fromCharCode, undefined, chunk);
    }
    return out;
  }
  H.TextDecoder_decode = (s, input, options) => {
    const view = bytesView(input);
    const stream = !!dictionary(options, 'TextDecodeOptions').stream;
    if (s.slot === 0) s.slot = n.decoderOpen(s.name, s.ignoreBOM);
    const text = n.decoderFeed(s.slot, latin1(view), !stream, s.fatal);
    if (!stream) s.slot = 0;
    if (text === undefined) {
      throw new TE('the input is not valid in this encoding');
    }
    return text;
  };

  // ------ atob and btoa (HTML §8.3) ------

  H.Window_btoa = (s, data) => {
    const r = n.base64Encode(data);
    if (r === undefined) throwDom('InvalidCharacterError', 'the string has a character above U+00FF');
    return r;
  };
  H.Window_atob = (s, data) => {
    const r = n.base64Decode(data);
    if (r === undefined) throwDom('InvalidCharacterError', 'the string is not valid base64');
    return r;
  };

  // ------ structuredClone (HTML §2.7.6) ------

  H.Window_structuredClone = (s, value, options) => {
    const d = dictionary(options, 'StructuredSerializeOptions');
    const transfer = d.transfer === undefined ? [] : seq(d.transfer, objectOf);
    const seen = new MapCtor();
    const m = transfer.length;
    for (let i = 0; i < m; i++) {
      const t = rdescriptor(transfer, i).value;
      if (!branded(abLength, t) || apply(mapHas, seen, [t]) || isDetached(t)) dataClone();
      apply(mapSet, seen, [t, null]);
    }
    for (let i = 0; i < m; i++) {
      const t = rdescriptor(transfer, i).value;
      apply(mapSet, seen, [t, cloneBuffer(t)]);
    }
    const out = cloneValue(value, seen);
    for (let i = 0; i < m; i++) apply(abTransfer, rdescriptor(transfer, i).value, []);
    return out;
  };

  // ------ navigator (HTML §8.11.1) ------
  //
  // Fixed values: nothing here may depend on the machine, the clock or the
  // version, or two renders of one document would differ. No network and no
  // cookies, so the two answers about those are false.

  const USER_AGENT = 'Mozilla/5.0 (compatible; surimono)';
  const languages = freeze(['en-US']);
  const pluginArray = make(I_PluginArray), mimeTypeArray = make(I_MimeTypeArray);
  const navigatorState = make(I_Navigator);
  H.Window_navigator = () => navigatorState.w;
  H.Window_clientInformation = () => navigatorState.w;
  H.Navigator_appCodeName = () => 'Mozilla';
  H.Navigator_appName = () => 'Netscape';
  H.Navigator_appVersion = () => '5.0 (compatible; surimono)';
  H.Navigator_platform = () => '';
  H.Navigator_product = () => 'Gecko';
  H.Navigator_productSub = () => '20030107';
  H.Navigator_userAgent = () => USER_AGENT;
  H.Navigator_vendor = () => '';
  H.Navigator_vendorSub = () => '';
  H.Navigator_language = () => 'en-US';
  H.Navigator_languages = () => languages;
  H.Navigator_onLine = () => false;
  H.Navigator_cookieEnabled = () => false;
  H.Navigator_plugins = () => pluginArray.w;
  H.Navigator_mimeTypes = () => mimeTypeArray.w;
  H.Navigator_pdfViewerEnabled = () => false;
  H.Navigator_hardwareConcurrency = () => 1;
  H.Navigator_maxTouchPoints = () => 0;
  H.Navigator_javaEnabled = () => false;
  H.PluginArray_length = () => 0;
  H.PluginArray_refresh = () => {};
  H.MimeTypeArray_length = () => 0;
  for (const id of [I_PluginArray, I_MimeTypeArray]) {
    defineProperty(protos[id], symbolIterator, { value: arrayValues, writable: true, configurable: true });
    op(id, 'item', function item() { self(this, id); return null; });
    op(id, 'namedItem', function namedItem() { self(this, id); return null; });
  }

  // ------ customElements (HTML §4.13.4) ------
  //
  // A registry only: names and constructors are kept and promises kept, and no
  // element is ever constructed or upgraded, so a `define` costs a page nothing
  // and gives it nothing. Reactions would need a hook in every path that adds a
  // node.

  const registry = make(I_CustomElementRegistry);
  registry.names = create(null);
  registry.byCtor = new MapCtor();
  registry.waiting = create(null);
  const RESERVED = create(null);
  for (const r of ['annotation-xml', 'color-profile', 'font-face', 'font-face-format',
                   'font-face-name', 'font-face-src', 'font-face-uri', 'missing-glyph']) {
    RESERVED[r] = true;
  }
  // HTML "valid custom element name".
  function validCustomName(name) {
    if (name.length === 0 || RESERVED[name] === true || apply(indexOf, name, ['-']) < 0) return false;
    const first = apply(charCodeAt, name, [0]);
    if (first < NAME_START || first > NAME_END) return false;
    for (let i = 1; i < name.length; i++) {
      const c = apply(charCodeAt, name, [i]);
      if ((c >= 65 && c <= 90) || c === 0 || c === 9 || c === 10 || c === 12 || c === 13 ||
          c === 32 || c === 47 || c === 62) {
        return false;
      }
    }
    return true;
  }
  // An HTML element name with an interface of its own, which is what `extends` takes.
  const unknownProto = () => protos[I_HTMLUnknownElement];
  const documentCreateNS = protos[I_Document].createElementNS;
  // It creates a detached element to ask, which a script could do itself. A name
  // createElementNS refuses (`1x`) is no element's, so not one to extend either.
  function knownElement(local) {
    const g = current();
    if (g === 0) return false;
    let e;
    try {
      e = apply(documentCreateNS, wrap(g, root()), ['http://www.w3.org/1999/xhtml', local]);
    } catch (x) {
      if (n.limit(x)) throw x;
      return false;
    }
    return getPrototypeOf(e) !== unknownProto();
  }
  H.Window_customElements = () => registry.w;
  H.CustomElementRegistry_define = (r, name, ctor, options) => {
    if (typeof ctor !== 'function') throw new TE('define: the constructor is not callable');
    if (!validCustomName(name)) throwDom('SyntaxError', `${name} is not a valid custom element name`);
    if (r.names[`$${name}`] !== undefined) {
      throwDom('NotSupportedError', `${name} is already defined`);
    }
    if (apply(mapHas, r.byCtor, [ctor])) {
      throwDom('NotSupportedError', 'this constructor is already defined');
    }
    // HTML §4.13.4 define, steps 7 and 14: `extends` must name a built-in HTML
    // element, and each lifecycle callback present must be callable.
    const extended = isObject(options) ? options.extends : undefined;
    if (extended !== undefined && extended !== null) {
      const local = `${extended}`;
      if (validCustomName(local) || !knownElement(local)) {
        throwDom('NotSupportedError', `${local} is not an element that can be extended`);
      }
    }
    const p = ctor.prototype;
    if (!isObject(p)) throw new TE('define: the constructor has no prototype object');
    for (const callback of ['connectedCallback', 'disconnectedCallback', 'connectedMoveCallback',
                            'adoptedCallback', 'attributeChangedCallback']) {
      const f = p[callback];
      if (f !== undefined && typeof f !== 'function') {
        throw new TE(`define: ${callback} is not callable`);
      }
    }
    r.names[`$${name}`] = ctor;
    apply(mapSet, r.byCtor, [ctor, name]);
    const waiting = r.waiting[`$${name}`];
    if (waiting !== undefined) {
      waiting.resolve(ctor);
      delete r.waiting[`$${name}`];
    }
  };
  H.CustomElementRegistry_get = (r, name) => r.names[`$${name}`];
  H.CustomElementRegistry_getName = (r, ctor) => {
    const name = apply(mapGet, r.byCtor, [ctor]);
    return name === undefined ? null : name;
  };
  H.CustomElementRegistry_whenDefined = (r, name) => {
    if (!validCustomName(name)) {
      try {
        throwDom('SyntaxError', `${name} is not a valid custom element name`);
      } catch (x) {
        return toPromiseRejected(x);
      }
    }
    const known = r.names[`$${name}`];
    if (known !== undefined) return toPromise(known);
    let w = r.waiting[`$${name}`];
    if (w === undefined) {
      w = create(null);
      w.promise = new PromiseCtor((resolve) => { w.resolve = resolve; });
      r.waiting[`$${name}`] = w;
    }
    return w.promise;
  };
  H.CustomElementRegistry_upgrade = () => {};
