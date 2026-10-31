// Timers that never fire, for libraries that schedule work they don't need
// for a static render.
//
// An island is rendered synchronously and its SVG taken at once, as in the
// libraries' own Node.js server-side rendering; anything a timer would do
// (an animation frame, a deferred tooltip) would happen after the snapshot,
// so it is never run. The ids are real numbers so `clearTimeout(id)` works.
// The phase-2 event loop with its virtual clock replaces this for page
// scripts.

let nextId = 1;

function define(name, fn) {
  if (typeof globalThis[name] !== 'function') {
    Object.defineProperty(globalThis, name, {
      value: fn,
      writable: true,
      enumerable: false,
      configurable: true,
    });
  }
}

define('setTimeout', function setTimeout() {
  return nextId++;
});
define('setInterval', function setInterval() {
  return nextId++;
});
define('clearTimeout', function clearTimeout() {});
define('clearInterval', function clearInterval() {});
