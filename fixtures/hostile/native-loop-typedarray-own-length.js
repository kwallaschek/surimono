// A typed array with its own length property of 2**40, shadowing the
// prototype's getter. Array.prototype.join reads the own property, so a guard
// that trusts typed arrays' lengths lets it through.
const ta = new Uint8Array(1);
Object.defineProperty(ta, "length", { value: 2 ** 40 });
Array.prototype.join.call(ta, "");
"finished";
