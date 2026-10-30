// A thenable whose `then` getter never returns. Only a real Promise may be
// awaited; if the host resolves thenables, the getter must still be stopped
// by the time limit.
({
  get then() {
    for (;;) {}
  },
});
