class A extends Array {}
const keep = Object.create(A.prototype);
new A();
