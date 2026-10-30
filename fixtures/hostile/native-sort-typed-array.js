// Sorting a 12,000,000-element Uint32Array (48 MB, under the heap limit): one
// long native call. The data is 2,930 copies of a pseudo-random 4,096-element
// block (filled with a fixed LCG, copied with the native set()), so the script
// itself is cheap and the time goes into the sort.
const block = new Uint32Array(4096);
let x = 12345;
for (let i = 0; i < block.length; i++) {
  x = (Math.imul(x, 1103515245) + 12345) >>> 0;
  block[i] = x;
}
const a = new Uint32Array(12000000);
for (let i = 0; i + block.length <= a.length; i += block.length) a.set(block, i);
a.sort();
"sorted";
