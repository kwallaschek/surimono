// Recursion through a getter: no visible call in the source.
const o = {
  get x() {
    return this.x;
  },
};
o.x;
