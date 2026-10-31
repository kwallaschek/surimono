// The engine fingerprint's canary (see embed.rs). Never run, only compiled:
// its bytecode is compared between the engine that wrote embedded bytecode
// and the engine about to read it. It touches every part of the bytecode
// format that could differ between two builds of the engine: the version
// byte, the numbering of opcodes (short and long forms) and of predefined
// atoms, function and variable flag layouts, closures, the constant pool
// (numbers, BigInts of several limbs, Latin-1 and UTF-16 strings, template
// objects), regular expression bytecode and class, generator and async
// function layouts. Changing this file changes every fingerprint.
'use strict';
var surimonoCanary = (function (global, undefinedValue) {
  const numbers = [0, -0, 1, -1, 127, 128, 255, 256, 32767, 32768, 65535, 65536,
    2147483647, -2147483648, 4294967295, 9007199254740993, 1.5, -2.25, 1e21, 1e-7,
    5e-324, 1.7976931348623157e308, NaN, Infinity, -Infinity, 0x7fffffff, 0o777, 0b1011];
  const bigints = [0n, 1n, -1n, 0xffffffffn, 0x100000000n, 0xffffffffffffffffn,
    0x10000000000000000n, -0x123456789abcdef0123456789abcdefn,
    123456789012345678901234567890123456789012345678901234567890n];
  const strings = ['', 'a', 'latin1 \u00e9\u00ff', 'utf16 \u3042\u30a2\u4e00',
    'astral \ud83d\ude00', 'lone \ud800', '\0\b\t\n\v\f\r"\'\\', 'x'.repeat(3)];
  const regexps = [/a|b*c+d?e{2,3}/g, /^(?<year>\d{4})-(?<m>\d\d)$/u, /[\p{L}\p{N}]+/u,
    /(?:x)(?=y)(?!z)(?<=w)(?<!v)\1?/y, /./s, /\bword\B/im, /[^\s\S]/d];
  function tag(s, ...v) { return s.raw.join('|') + v.length; }
  const templates = [`plain`, `a${1}b${'c'}d`, tag`x${2}y\u0041${3}`];
  let counter = 0;
  const closure = () => { counter += 1; return counter; };
  function* generator(n) { for (let i = 0; i < n; i++) { const r = yield i; if (r) return r; } }
  async function asyncFn(p) { try { return await p; } catch (e) { throw e; } finally { counter--; } }
  async function* asyncGenerator() { for await (const x of [1, 2]) yield x; }
  class Base {
    #secret = 1;
    static #count = 0;
    static { Base.#count = 1; }
    constructor(a = 1, ...rest) { this.a = a; this.rest = rest; new.target; }
    get value() { return this.#secret; }
    set value(v) { this.#secret = v; }
    static count() { return Base.#count; }
    #privateMethod() { return #secret in this; }
    [Symbol.iterator]() { return [this.a][Symbol.iterator](); }
  }
  class Derived extends Base {
    constructor() { super(2, 3, 4); }
    get value() { return super.value + 1; }
  }
  function sloppyish(o) {
    const { a, b: { c = 2 } = {}, ...others } = o ?? {};
    const [x, , y = 3, ...zs] = [1, 2, undefinedValue, 4, 5];
    let total = a?.length ?? 0;
    total ||= 1; total &&= 2; total ??= 3;
    total **= 2; total <<= 1; total >>>= 1; total |= 4; total ^= 1; total %= 7;
    label: for (const k in others) { for (const v of zs) { if (v > 4) continue label; if (v < 0) break label; } }
    switch (typeof x) { case 'number': total += x; break; default: total -= y; }
    try { throw new RangeError('r'); } catch ({ message }) { total += message.length; }
    delete others.z; void 0;
    return [total, x, y, c, typeof others, x instanceof Object, 'a' in others, (1, 2)];
  }
  const object = {
    __proto__: null, plain: 1, 'quoted key': 2, [Symbol.toPrimitive]: () => 3,
    method() { return arguments.length; }, get g() { return 4; }, set s(v) {},
    async am() {}, *gm() {}, 42: 'n', 1.5: 'f',
  };
  return {
    numbers, bigints, strings, regexps, templates, closure, generator, asyncFn,
    asyncGenerator, Base, Derived, sloppyish, object,
    builtins: [global.Object, Array.prototype.map, JSON.stringify, Math.max, Promise.all,
      Reflect.ownKeys, Proxy, Map, Set, WeakMap, Symbol.asyncIterator, Date.UTC,
      String.raw, Number.isInteger, BigInt.asUintN, ArrayBuffer, DataView, Float64Array,
      Error.prototype.toString, encodeURIComponent, globalThis.constructor, eval],
  };
})(globalThis);
