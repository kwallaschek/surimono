// A length getter that answers 1 on its first read and 2**40 on every later
// one. ECMA-262 reads length once (LengthOfArrayLike), so a conforming join
// returns "" at once; a guard that reads length itself before calling the
// native join sees 1, and the native loop then sees 2**40.
let reads = 0;
const o = {
  get length() {
    reads++;
    return reads === 1 ? 1 : 2 ** 40;
  },
};
Array.prototype.join.call(o, "");
"finished";
