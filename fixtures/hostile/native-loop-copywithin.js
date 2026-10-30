// copyWithin moves 2**40 - 1 elements natively in an array-like.
Array.prototype.copyWithin.call({ length: 2 ** 40 }, 0, 1);
"finished";
