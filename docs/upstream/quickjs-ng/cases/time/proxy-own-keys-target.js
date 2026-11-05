const k = [], t = {};
for (let i = 0; i < 100000; i++) { k.push('k' + i); t['k' + i] = 1; }
const p = new Proxy(Object.freeze(t), { ownKeys: () => k });
k.reverse();
armDeadline(); Reflect.ownKeys(p);
