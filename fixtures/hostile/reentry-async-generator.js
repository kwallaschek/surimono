// next() on an async generator runs its body at once and turns a throw into a
// rejected promise; the outer loop asks again.
async function* spin() {
  for (;;) {}
}
for (;;) {
  spin().next();
}
