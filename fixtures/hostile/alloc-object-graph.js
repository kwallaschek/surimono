// Allocation bomb made of small objects, so no single allocation is large.
let head = null;
for (;;) head = { next: head, a: 1, b: 2, c: 3 };
