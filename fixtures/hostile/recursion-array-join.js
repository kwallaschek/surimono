// Native recursion through Array.prototype.join/toString on nested arrays.
let a = [];
for (let i = 0; i < 100000; i++) a = [a];
String(a);
"joined";
