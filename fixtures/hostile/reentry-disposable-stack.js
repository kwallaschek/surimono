// DisposableStack#dispose runs every deferred callback and collects their
// errors into a SuppressedError. An infinite loop inside a callback, re-entered
// by an outer loop that catches whatever dispose throws. (Where DisposableStack
// is missing, the ReferenceError is caught too and the outer loop spins.)
for (;;) {
  try {
    const stack = new DisposableStack();
    stack.defer(() => {
      for (;;) {}
    });
    stack.dispose();
  } catch {}
}
