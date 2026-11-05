// The same key 200,000 times.
var t = {}, k = [], d = [], n = [];
for (var i = 0; i < 200000; i++) { t["k" + i] = 1; k.push("k" + i); d.push("x"); n.push(String(199999 - i)); }
var p = new Proxy({}, { ownKeys: () => d });
for (;;) try { Reflect.ownKeys(p); } catch (e) {}
