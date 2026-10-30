// A 200,000-long linked list (about 16 MB), then dropped. Freeing it must not recurse
// in the engine (no crash), and the script finishes normally.
let head = null;
for (let i = 0; i < 200000; i++) head = { next: head };
head = null;
"freed";
