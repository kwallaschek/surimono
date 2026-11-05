// A frozen target: every key of the target must be in the result.
var t = {}, k = [], d = [], n = [];
for (var i = 0; i < 200000; i++) { t["k" + i] = 1; k.push("k" + i); d.push("x"); n.push(String(199999 - i)); }
var p = new Proxy(Object.freeze(t), { ownKeys: () => k });
for (;;) Reflect.ownKeys(p);
