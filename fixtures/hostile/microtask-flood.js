// Every microtask queues the next one, forever. The completion value is a
// plain string, but renders always drain the job queue, and that drain must
// stop at the time limit.
function f() {
  Promise.resolve().then(f);
}
f();
"queued";
