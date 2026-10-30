// Every microtask queues two more: the queue grows exponentially.
function f() {
  Promise.resolve().then(f);
  Promise.resolve().then(f);
}
f();
"queued";
