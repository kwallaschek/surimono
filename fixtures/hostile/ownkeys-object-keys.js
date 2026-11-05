// Object.keys through a proxy: the same check, then a descriptor per key.
var t = {}, k = [], d = [], n = [];
for (var i = 0; i < 200000; i++) { t["k" + i] = 1; k.push("k" + i); d.push("x"); n.push(String(199999 - i)); }
var p = new Proxy({}, { ownKeys: () => k });
for (;;) Object.keys(p);
