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
})
