// A proxy's ownKeys result is checked in C after the trap has returned, in every
// path through a proxy's own keys: 200,000 keys per call, in a loop, so the case
// ends at the time limit if each call is short, and overruns if one is long.
var t = {}, k = [], d = [], n = [];
for (var i = 0; i < 200000; i++) { t["k" + i] = 1; k.push("k" + i); d.push("x"); n.push(String(199999 - i)); }
var p = new Proxy({}, { ownKeys: () => k });
for (;;) Reflect.ownKeys(p);
