// An infinite loop that tries to swallow the interrupt and keep going.
// The time-limit interrupt must not be catchable (or must fire again at once).
for (;;) {
  try {
    for (;;) {}
  } catch (e) {
    // ignore and loop again
  } finally {
    // a finally block must not get to restart the loop either
  }
}
