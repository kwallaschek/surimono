// length is an object whose valueOf answers 2**40: ToLength converts it, so a
// guard that only checks numeric lengths lets it through.
Array.prototype.join.call({ length: { valueOf: () => 2 ** 40 } }, "");
"finished";
