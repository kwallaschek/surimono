// Allocation bomb: keep every array alive until the heap is full.
const hoard = [];
for (;;) hoard.push(new Array(1 << 16).fill(1.5));
