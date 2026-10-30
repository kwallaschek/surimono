// A built-in that loops natively over an array-like: join walks 2**40 indices
// of an object with no elements. The loop runs in C, not in bytecode, so an
// interrupt handler that only polls between bytecodes never sees it. Any typed
// error within the limits is safe; a hang is not.
Array.prototype.join.call({ length: 2 ** 40 }, "");
"finished";
