// Many 1 MiB typed arrays, all kept alive.
const keep = [];
for (;;) keep.push(new Float64Array(1 << 17));
