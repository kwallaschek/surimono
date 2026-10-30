// Native (C) recursion: JSON.stringify of a 100,000-deep array. The stack
// limit must also cover recursion inside the engine, or the process crashes.
let a = [];
for (let i = 0; i < 100000; i++) a = [a];
JSON.stringify(a);
"stringified";
