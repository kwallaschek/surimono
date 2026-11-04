class A { #p = 1; static #q = 2; m() { return this.#p; } static n() { return A.#q; } }
class B extends A { constructor() { super(); } }
for (const x of [1, 2]) { x; }
for (const k in { a: 1 }) { k; }
function g({ a = 1, b = 2 } = {}, [c = 3, d = 4] = []) { return a + b + c + d; }
switch (1) { case 1: break; default: g(); }
