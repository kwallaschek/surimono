// Promise.try calls its callback at once and turns a throw into a rejection:
// the same swallow-and-re-enter shape as the executor case.
for (;;) {
  Promise.try(() => {
    for (;;) {}
  });
}
