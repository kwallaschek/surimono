armDeadline(); const o = Object.create(new Uint8Array(0)); Object.defineProperty(o, 'length', { value: 2 ** 32 }); Array.prototype.fill.call(o, 0)
