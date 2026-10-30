// Allocation bomb: double a string until it no longer fits.
let s = "x";
for (;;) s += s;
