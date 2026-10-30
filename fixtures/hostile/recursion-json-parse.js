// Native recursion in the JSON parser: 100,000 nested arrays. An engine with an
// iterative parser may finish; that is fine too.
const n = 100000;
JSON.parse("[".repeat(n) + "]".repeat(n));
"parsed";
