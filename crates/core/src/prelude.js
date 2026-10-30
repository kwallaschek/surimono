// surimono sandbox prelude. Runs once in every new context, before any
// script, as a function of one config object. Keep it small and free of
// observable globals: it is part of every render.
(function (config) {
  'use strict';

  const apply = Reflect.apply;
  const defineProperty = Object.defineProperty;
  const isArray = Array.isArray;
  const isView = ArrayBuffer.isView;
  const stringValueOf = String.prototype.valueOf;
  const RangeErrorCtor = RangeError;
  const maxLength = config.maxLength;

  // --- Long native loops --------------------------------------------------
  //
  // Some of QuickJS's array built-ins walk an array-like index by index in
  // native code without checking for interrupts, so
  // `Array.prototype.join.call({ length: 2 ** 40 })` would never return and
  // the time limit could not stop it. No array-like can hold more elements
  // than fit in the heap, so a longer one is sparse by construction: refuse
  // it before the native loop starts. Typed arrays and strings use their
  // real length and are exempt.

  function isStringObject(o) {
    try {
      apply(stringValueOf, o, []);
      return true;
    } catch (e) {
      return false;
    }
  }

  function checkLength(o, what) {
    if (o === null || (typeof o !== 'object' && typeof o !== 'function')) {
      return;
    }
    const len = +o.length;
    if (len > maxLength && !isView(o) && !isStringObject(o)) {
      throw new RangeErrorCtor(
        what + ': length ' + len + ' exceeds the sandbox limit of ' + maxLength,
      );
    }
  }

  function replace(target, name, fn, length) {
    defineProperty(fn, 'length', { value: length, configurable: true });
    defineProperty(target, name, {
      value: fn,
      writable: true,
      enumerable: false,
      configurable: true,
    });
  }

  function guardThis(target, name) {
    const native = target[name];
    const what = 'Array.prototype.' + name;
    const guarded = {
      [name](...args) {
        checkLength(this, what);
        return apply(native, this, args);
      },
    }[name];
    replace(target, name, guarded, native.length);
  }

  const AP = Array.prototype;
  for (const name of [
    'copyWithin', 'join', 'reverse', 'shift', 'slice', 'sort', 'splice',
    'toLocaleString', 'unshift',
  ]) {
    guardThis(AP, name);
  }

  // concat spreads its arguments too.
  const nativeConcat = AP.concat;
  const isConcatSpreadable = Symbol.isConcatSpreadable;
  replace(AP, 'concat', {
    concat(...items) {
      checkLength(this, 'Array.prototype.concat');
      for (let i = 0; i < items.length; i++) {
        const item = items[i];
        if (item !== null && typeof item === 'object' &&
            (isArray(item) || item[isConcatSpreadable])) {
          checkLength(item, 'Array.prototype.concat');
        }
      }
      return apply(nativeConcat, this, items);
    },
  }.concat, nativeConcat.length);

  // flat descends into nested arrays, which the receiver check can't see:
  // check them, to the requested depth, before the native call. (This loop
  // is script code, so the time limit applies to it.)
  function scan(arr, depth, what) {
    checkLength(arr, what);
    if (depth < 1) return;
    const len = +arr.length;
    for (let i = 0; i < len; i++) {
      const el = arr[i];
      if (isArray(el)) scan(el, depth - 1, what);
    }
  }
  const nativeFlat = AP.flat;
  replace(AP, 'flat', {
    flat(depth) {
      let d = depth === undefined ? 1 : +depth;
      if (d !== d) d = 0;
      if (this !== null && this !== undefined) scan(this, d, 'Array.prototype.flat');
      return apply(nativeFlat, this, arguments);
    },
  }.flat, nativeFlat.length);

  // flatMap flattens what the callback returns: check each result.
  const nativeFlatMap = AP.flatMap;
  replace(AP, 'flatMap', {
    flatMap(callback, thisArg) {
      checkLength(this, 'Array.prototype.flatMap');
      if (typeof callback !== 'function') {
        return apply(nativeFlatMap, this, arguments);
      }
      const checked = function (el, i, arr) {
        const r = apply(callback, thisArg, [el, i, arr]);
        if (isArray(r)) checkLength(r, 'Array.prototype.flatMap');
        return r;
      };
      return apply(nativeFlatMap, this, [checked]);
    },
  }.flatMap, nativeFlatMap.length);

  // JSON.stringify walks an array replacer natively.
  const nativeStringify = JSON.stringify;
  replace(JSON, 'stringify', {
    stringify(value, replacer, space) {
      if (isArray(replacer)) checkLength(replacer, 'JSON.stringify');
      return apply(nativeStringify, JSON, arguments);
    },
  }.stringify, nativeStringify.length);

  // String.raw walks its template's raw strings.
  const nativeRaw = String.raw;
  replace(String, 'raw', {
    raw(template, ...subs) {
      if (template !== null && template !== undefined) {
        checkLength(template.raw, 'String.raw');
      }
      return apply(nativeRaw, String, arguments);
    },
  }.raw, nativeRaw.length);

  // --- Determinism -----------------------------------------------------------
  //
  // Nothing a script can read may depend on the host: Math.random is seeded,
  // the clock is fixed, local time is a fixed offset from UTC (QuickJS would
  // otherwise read the host's zone), and weak references never empty.

  const imul = Math.imul;
  const construct = Reflect.construct;
  const regexTest = RegExp.prototype.test;
  const TypeErrorCtor = TypeError;
  const symbolKeyFor = Symbol.keyFor;
  const nowMs = config.nowMs;
  const offsetMinutes = config.utcOffsetMinutes;
  const offsetMs = offsetMinutes * 60000;

  // Math.random: xoshiro128** seeded through splitmix32, 53-bit results.
  function splitmix32(a) {
    return function () {
      a = (a + 0x9e3779b9) | 0;
      let t = a ^ (a >>> 16);
      t = imul(t, 0x21f0aaad);
      t = t ^ (t >>> 15);
      t = imul(t, 0x735a2d97);
      return (t ^ (t >>> 15)) | 0;
    };
  }
  function rotl(x, k) {
    return (x << k) | (x >>> (32 - k));
  }
  const seedLo = splitmix32(config.seedLo | 0);
  const seedHi = splitmix32(config.seedHi | 0);
  let s0 = seedLo(), s1 = seedHi(), s2 = seedLo(), s3 = seedHi();
  if ((s0 | s1 | s2 | s3) === 0) s0 = 1;
  function next32() {
    const r = imul(rotl(imul(s1, 5), 7), 9);
    const t = s1 << 9;
    s2 ^= s0;
    s3 ^= s1;
    s1 ^= s2;
    s0 ^= s3;
    s2 ^= t;
    s3 = rotl(s3, 11);
    return r >>> 0;
  }
  replace(Math, 'random', {
    random() {
      const hi = next32() >>> 5;
      const lo = next32() >>> 6;
      return (hi * 67108864 + lo) / 9007199254740992;
    },
  }.random, 0);

  // Date: fixed "now", local time = UTC + offsetMinutes.
  const RealDate = Date;
  const DP = RealDate.prototype;
  const nativeGetTime = DP.getTime;
  const nativeSetTime = DP.setTime;
  const nativeParse = RealDate.parse;
  const nativeUTC = RealDate.UTC;
  const nativeTimezoneOffset = DP.getTimezoneOffset;
  const utc = {};
  for (const name of ['FullYear', 'Month', 'Date', 'Day', 'Hours', 'Minutes',
                      'Seconds', 'Milliseconds']) {
    utc['get' + name] = DP['getUTC' + name];
    if (name !== 'Day') utc['set' + name] = DP['setUTC' + name];
  }
  // One scratch Date, set to the local wall-clock instant and read with the
  // UTC accessors. Never escapes and never runs script code.
  const scratch = new RealDate(0);

  function timeOf(d) {
    return apply(nativeGetTime, d, []);
  }
  function atLocal(t) {
    apply(nativeSetTime, scratch, [t + offsetMs]);
    return scratch;
  }
  function field(local, name) {
    return apply(utc['get' + name], local, []);
  }

  for (const name of ['FullYear', 'Month', 'Date', 'Day', 'Hours', 'Minutes',
                      'Seconds', 'Milliseconds']) {
    const getter = utc['get' + name];
    replace(DP, 'get' + name, {
      ['get' + name]() {
        const t = timeOf(this);
        if (t !== t) return NaN;
        return apply(getter, atLocal(t), []);
      },
    }['get' + name], 0);
  }

  for (const name of ['FullYear', 'Month', 'Date', 'Hours', 'Minutes',
                      'Seconds', 'Milliseconds']) {
    const setter = utc['set' + name];
    const length = DP['set' + name].length;
    replace(DP, 'set' + name, {
      ['set' + name](...args) {
        const t = timeOf(this);
        // setFullYear on an invalid date starts from local midnight 1970.
        const local = t === t ? t + offsetMs : (name === 'FullYear' ? 0 : NaN);
        apply(nativeSetTime, scratch, [local]);
        apply(setter, scratch, args);
        return apply(nativeSetTime, this, [timeOf(scratch) - offsetMs]);
      },
    }['set' + name], length);
  }

  if (typeof DP.getYear === 'function') {
    replace(DP, 'getYear', {
      getYear() {
        const t = timeOf(this);
        return t !== t ? NaN : field(atLocal(t), 'FullYear') - 1900;
      },
    }.getYear, 0);
  }
  if (typeof DP.setYear === 'function') {
    const setFullYear = DP.setFullYear;
    const trunc = Math.trunc;
    replace(DP, 'setYear', {
      setYear(year) {
        timeOf(this);
        const y = +year;
        if (y !== y) return apply(nativeSetTime, this, [NaN]);
        const yi = trunc(y);
        return apply(setFullYear, this, [yi >= 0 && yi <= 99 ? 1900 + yi : y]);
      },
    }.setYear, 1);
  }

  replace(DP, 'getTimezoneOffset', {
    getTimezoneOffset() {
      const t = timeOf(this);
      return t !== t ? NaN : 0 - offsetMinutes;
    },
  }.getTimezoneOffset, 0);

  // Formatting, matching Chrome's output for en-US.
  const dayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
  const monthNames = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug',
                      'Sep', 'Oct', 'Nov', 'Dec'];
  function pad(n, width) {
    let s = '' + n;
    while (s.length < width) s = '0' + s;
    return s;
  }
  // ECMA-262 DateString: sign, then at least four digits.
  function yearString(y) {
    return y < 0 ? '-' + pad(-y, 4) : pad(y, 4);
  }
  function zoneString() {
    const a = offsetMinutes < 0 ? -offsetMinutes : offsetMinutes;
    return 'GMT' + (offsetMinutes < 0 ? '-' : '+') + pad((a / 60) | 0, 2) +
      pad(a % 60, 2) + (offsetMinutes === 0 ? ' (Coordinated Universal Time)' : '');
  }
  function dateString(t) {
    const l = atLocal(t);
    return dayNames[field(l, 'Day')] + ' ' + monthNames[field(l, 'Month')] + ' ' +
      pad(field(l, 'Date'), 2) + ' ' + yearString(field(l, 'FullYear'));
  }
  function timeString(t) {
    const l = atLocal(t);
    return pad(field(l, 'Hours'), 2) + ':' + pad(field(l, 'Minutes'), 2) + ':' +
      pad(field(l, 'Seconds'), 2) + ' ' + zoneString();
  }
  function localeDate(t) {
    const l = atLocal(t);
    return (field(l, 'Month') + 1) + '/' + field(l, 'Date') + '/' + field(l, 'FullYear');
  }
  function localeTime(t) {
    const l = atLocal(t);
    const h = field(l, 'Hours');
    return ((h % 12) || 12) + ':' + pad(field(l, 'Minutes'), 2) + ':' +
      pad(field(l, 'Seconds'), 2) + ' ' + (h < 12 ? 'AM' : 'PM');
  }
  function fullString(t) {
    return t !== t ? 'Invalid Date' : dateString(t) + ' ' + timeString(t);
  }
  function formatter(name, fn) {
    replace(DP, name, {
      [name]() {
        const t = timeOf(this);
        return t !== t ? 'Invalid Date' : fn(t);
      },
    }[name], 0);
  }
  formatter('toString', fullString);
  formatter('toDateString', dateString);
  formatter('toTimeString', timeString);
  formatter('toLocaleString', (t) => localeDate(t) + ', ' + localeTime(t));
  formatter('toLocaleDateString', localeDate);
  formatter('toLocaleTimeString', localeTime);

  // Date.parse reads strings without a zone as local time, in the host's
  // zone. Detect those and move them to ours.
  const explicitZone =
    /\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?\s*(?:Z|[+-]\d{2}(?::?\d{2})?)\b|\b(?:GMT|UTC|UT)\b/i;
  const isoDateOnly = /^\s*[+-]?\d{4,6}(?:-\d{2}(?:-\d{2})?)?\s*$/;
  function parse(input) {
    const s = `${input}`;
    const t = apply(nativeParse, RealDate, [s]);
    if (t !== t || apply(regexTest, isoDateOnly, [s]) || apply(regexTest, explicitZone, [s])) {
      return t;
    }
    apply(nativeSetTime, scratch, [t]);
    const hostOffsetMs = apply(nativeTimezoneOffset, scratch, []) * 60000;
    return t - hostOffsetMs - offsetMs;
  }

  const DateShim = function Date(year, month, date, hours, minutes, seconds, ms) {
    if (new.target === undefined) return fullString(nowMs);
    const n = arguments.length;
    let v;
    if (n === 0) {
      v = nowMs;
    } else if (n === 1) {
      if (typeof year !== 'string') return construct(RealDate, [year], new.target);
      v = parse(year);
    } else {
      v = apply(nativeUTC, undefined, arguments) - offsetMs;
    }
    return construct(RealDate, [v], new.target);
  };
  defineProperty(DateShim, 'prototype', {
    value: DP, writable: false, enumerable: false, configurable: false,
  });
  replace(DateShim, 'now', { now() { return nowMs; } }.now, 0);
  replace(DateShim, 'parse', { parse(s) { return parse(s); } }.parse, 1);
  replace(DateShim, 'UTC', nativeUTC, nativeUTC.length);
  defineProperty(DP, 'constructor', {
    value: DateShim, writable: true, enumerable: false, configurable: true,
  });
  defineProperty(globalThis, 'Date', {
    value: DateShim, writable: true, enumerable: false, configurable: true,
  });

  // performance: a virtual clock that stands still (timers come with the
  // phase-2 event loop).
  const performance = {};
  replace(performance, 'now', { now() { return 0; } }.now, 0);
  defineProperty(performance, 'timeOrigin', {
    value: nowMs, writable: false, enumerable: true, configurable: true,
  });
  defineProperty(globalThis, 'performance', {
    value: performance, writable: true, enumerable: true, configurable: true,
  });

  // WeakRef and FinalizationRegistry depend on when the GC runs, which
  // depends on earlier renders in the same runtime. Make them strong: deref
  // always returns the target and cleanup callbacks never run. The context
  // is thrown away after the render, so nothing is kept for long.
  function canBeHeldWeakly(v) {
    return (typeof v === 'object' && v !== null) || typeof v === 'function' ||
      (typeof v === 'symbol' && apply(symbolKeyFor, Symbol, [v]) === undefined);
  }
  class WeakRef {
    #target;
    constructor(target) {
      if (!canBeHeldWeakly(target)) {
        throw new TypeErrorCtor('WeakRef: invalid target');
      }
      this.#target = target;
    }
    deref() {
      return this.#target;
    }
  }
  class FinalizationRegistry {
    #cleanup;
    constructor(cleanup) {
      if (typeof cleanup !== 'function') {
        throw new TypeErrorCtor('FinalizationRegistry: cleanup must be callable');
      }
      this.#cleanup = cleanup;
    }
    register(target, heldValue, token) {
      this.#cleanup;
      if (!canBeHeldWeakly(target)) {
        throw new TypeErrorCtor('FinalizationRegistry.register: invalid target');
      }
      if (target === heldValue) {
        throw new TypeErrorCtor('FinalizationRegistry.register: target and held value are the same');
      }
      if (token !== undefined && !canBeHeldWeakly(token)) {
        throw new TypeErrorCtor('FinalizationRegistry.register: invalid token');
      }
    }
    unregister(token) {
      this.#cleanup;
      if (!canBeHeldWeakly(token)) {
        throw new TypeErrorCtor('FinalizationRegistry.unregister: invalid token');
      }
      return false;
    }
  }
  for (const [name, cls] of [['WeakRef', WeakRef],
                             ['FinalizationRegistry', FinalizationRegistry]]) {
    defineProperty(cls.prototype, Symbol.toStringTag, {
      value: name, writable: false, enumerable: false, configurable: true,
    });
    defineProperty(globalThis, name, {
      value: cls, writable: true, enumerable: false, configurable: true,
    });
  }

  // A silent console, so library warnings don't throw ReferenceErrors.
  const console = {};
  for (const name of ['debug', 'error', 'info', 'log', 'trace', 'warn']) {
    replace(console, name, { [name]() {} }[name], 0);
  }
  defineProperty(globalThis, 'console', {
    value: console, writable: true, enumerable: false, configurable: true,
  });
})
