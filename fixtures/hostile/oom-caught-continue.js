// Catches the out-of-memory error, drops the garbage and carries on as if
// nothing happened. Policy: a render that hit the memory limit fails with
// MemoryLimit even if the script caught the error.
let survived = "no";
try {
  const hoard = [];
  for (;;) hoard.push(new Array(1 << 16).fill(0));
} catch (e) {
  survived = "yes";
}
survived;
