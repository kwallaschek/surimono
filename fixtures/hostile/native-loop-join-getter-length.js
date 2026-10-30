// join over an array-like whose length comes from a getter: the length is
// read once through JavaScript, then the loop is native.
Array.prototype.join.call(
  {
    get length() {
      return 2 ** 40;
    },
  },
  "",
);
"finished";
