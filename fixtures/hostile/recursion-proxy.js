// Recursion through a Proxy trap that reads from its own proxy.
const p = new Proxy(
  {},
  {
    get(target, key, receiver) {
      return receiver[key];
    },
  },
);
p.anything;
