// Recursion through implicit conversion: String(o) calls toString, which
// converts `this` again.
const o = {
  toString() {
    return "<" + String(this) + ">";
  },
};
String(o);
