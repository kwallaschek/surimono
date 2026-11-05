// Integer-like keys in descending order, the atoms a script can choose to attack
// a hash or a sort.
var t = {}, k = [], d = [], n = [];
for (var i = 0; i < 200000; i++) { t["k" + i] = 1; k.push("k" + i); d.push("x"); n.push(String(199999 - i)); }
var p = new Proxy({}, { ownKeys: () => n });
for (;;) Reflect.ownKeys(p);
