// A DataView (no length of its own in the spec) given a length of 2**40.
const dv = new DataView(new ArrayBuffer(8));
dv.length = 2 ** 40;
Array.prototype.join.call(dv, "");
"finished";
