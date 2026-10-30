// reverse swaps elements natively across a 2**40-long array-like.
Array.prototype.reverse.call({ length: 2 ** 40 });
"finished";
