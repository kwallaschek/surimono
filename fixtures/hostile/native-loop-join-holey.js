// join over a real array of the maximum length (2**32 - 1) with no elements:
// about four billion native iterations producing an empty string.
new Array(2 ** 32 - 1).join("");
"finished";
