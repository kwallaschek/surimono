
'use strict';
class Base {
  static #count = 0;
  #secret;
  static { Base.ready = true; }
  constructor(x = 1, { y, z: [w = 2] = [] } = {}) { this.#secret = x + (y ?? 0) + w; Base.#count++; }
  get secret() { return this.#secret; }
  set secret(v) { this.#secret = v; }
  #hidden() { return this.#secret * 2; }
  reveal() { return this.#hidden(); }
  static count() { return Base.#count; }
  *[Symbol.iterator]() { yield this.#secret; yield* [1, 2]; }
}
class Derived extends Base {
  field = 3;
  ['computed' + 1] = 4;
  constructor(...args) { super(...args); this.extra = new.target.name; }
  reveal() { return super.reveal() + this.field; }
  static async *gen(n) { for (let i = 0; i < n; i++) { await null; yield i; } }
}
function sloppy() { return typeof this; }
const tag = (s, ...v) => s.raw.join('|') + v.join(',');
let acc = 0n;
outer: for (const [k, v] of Object.entries({ a: 1, b: 2, c: 3 })) {
  for (const key in { p: 1, q: 2 }) {
    if (key === 'q') continue outer;
    switch (v) {
      case 1: acc += 1n; break;
      case 2: { acc **= 2n; break; }
      default: acc ||= 5n;
    }
  }
}
let n = 0;
do { n++; } while (n < 3);
while (n > 0) { try { if (n === 2) throw new RangeError('r'); } catch ({ message }) { n -= message.length; } finally { n--; } }
const d = new Derived(5, { y: 1 });
const o = { a: { b: [1, { c: 2 }] }, get g() { return 7; }, set s(v) { this._s = v; }, m() { return this?.a?.b?.[1]?.c; }, async am() { return 1; }, *gm() { yield 1; } };
const { a: { b: [, { c } = {}] } = {}, ...rest } = o;
let x1 = null; x1 ??= 4; let x2 = 0; x2 ||= 5; let x3 = 1; x3 &&= 6;
const re = /(?<year>\d{4})-(?<m>\d\d)/gu;
const dates = [...'2026-10 2027-01'.matchAll(re)].map((mm) => mm.groups.year + mm.groups.m);
const fns = [];
for (let i = 0; i < 3; i++) fns.push(() => i);
const label = `${tag`a${1}b${2}c`}:${dates}:${fns.map((f) => f())}`;
const big = 1_000_000n * 3n;
const ev = eval('(function inner(p) { var q = p + 1; return eval("q * 2"); })(20)');
const fn = new Function('a', 'b = 2', 'return a ** b + arguments.length');
function sum3(a, b, c) { return this.k + a + b + c; }
const bound0 = sum3.bind({ k: 1 });
const bound2 = sum3.bind({ k: 2 }, 3, 4);
const bargs = new Array(300).fill(1);
const boundMany = sum3.bind({ k: 3 }, ...bargs);
function Pt(x) { this.x = x; }
const BoundPt = Pt.bind(null, 7);
(async () => {
  const out = [];
  for await (const v of Derived.gen(3)) out.push(v);
  try { await Promise.reject(new Error('x')); } catch (e) { out.push(e.message); }
  const r = await Promise.allSettled([o.am(), Promise.resolve(2)]);
  out.push(await Promise.any([Promise.reject(new Error('y')), Promise.resolve(4), 5]));
  globalThis.asyncResult = out.join() + r.length;
})();
[d.secret, d.reveal(), Base.count(), [...d].join(), d.extra, d.computed1, sloppy.call(1), String(acc), n,
 c, Object.keys(rest).length, o.m(), o.g, x1 + x2 + x3, label, String(big), ev, fn(3), [...o.gm()].length,
 bound0(1, 2, 3), bound2(5), boundMany(), boundMany.length, new BoundPt().x, BoundPt.name,
 typeof Base.ready, (function () { return arguments.length; })(1, 2, 3),
 JSON.stringify({ k: [1, 'two', { three: 3 }] }, null, 2).length,
 new Map([[1, { deep: [new Set([1])] }]]).size,
 Array.from({ length: 5 }, (_, i) => i * i).reduce((s, v) => s + v, 0),
].join(';');
