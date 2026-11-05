const k = [];
for (let i = 0; i < 200000; i++) k.push('k' + i);
const p = new Proxy({}, { ownKeys: () => k });
armDeadline(); Reflect.ownKeys(p);
