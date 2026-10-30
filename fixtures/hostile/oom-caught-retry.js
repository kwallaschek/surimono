// Catches the out-of-memory error and retries forever, holding on to
// everything it already allocated. Memory exhaustion must end the render.
const hoard = [];
for (;;) {
  try {
    hoard.push(new Array(1 << 16).fill(0));
  } catch (e) {
    // swallow and retry
  }
}
