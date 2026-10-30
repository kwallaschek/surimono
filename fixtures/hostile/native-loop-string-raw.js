// String.raw walks raw.length (2**40) template pieces natively.
String.raw({ raw: { length: 2 ** 40 } });
"finished";
