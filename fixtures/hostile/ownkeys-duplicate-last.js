// A duplicate at the end: the check throws, after looking at every key, and the
// script catches it and goes again.
var t = {}, k = [], d = [], n = [];
for (var i = 0; i < 200000; i++) { t["k" + i] = 1; k.push("k" + i); d.push("x"); n.push(String(199999 - i)); }
var a = k.concat("k0"), p = new Proxy({}, { ownKeys: () => a });
for (;;) try { Reflect.ownKeys(p); } catch (e) {}
