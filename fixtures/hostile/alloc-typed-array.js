// One huge allocation, 1 GiB, far beyond the 64 MiB limit.
new Uint8Array(1 << 30).length;
