// Error.prepareStackTrace (a V8 API that QuickJS-ng also has) is called by the
// engine while it builds an error's stack, possibly while building the
// interrupt error itself. An infinite loop inside it, re-entered by an outer
// loop that catches whatever comes out.
Error.prepareStackTrace = () => {
  for (;;) {}
};
for (;;) {
  try {
    new Error("x").stack;
  } catch {}
}
