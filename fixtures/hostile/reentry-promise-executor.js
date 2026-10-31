// The Promise constructor turns anything its executor throws into a
// rejection. If the time-limit interrupt is treated like any other exception
// there, it is swallowed and the outer loop starts the next infinite loop.
for (;;) {
  new Promise(() => {
    for (;;) {}
  });
}
