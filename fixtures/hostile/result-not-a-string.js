// The result is an object whose toString never returns. The host must not
// call back into JS to convert a result: a non-string result is an error.
({
  toString() {
    for (;;) {}
  },
  valueOf() {
    for (;;) {}
  },
});
