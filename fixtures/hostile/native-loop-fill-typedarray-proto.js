// Array.prototype.fill over an ordinary object that inherits from
// Uint8Array.prototype and has its own length of 2**40: it looks like a typed
// array to a prototype check, but fill writes 2**40 ordinary properties.
const o = Object.create(Uint8Array.prototype, { length: { value: 2 ** 40, writable: true } });
Array.prototype.fill.call(o, 7);
"finished";
