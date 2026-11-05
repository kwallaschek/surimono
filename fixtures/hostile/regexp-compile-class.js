// A class of 65,536 single code points, folded one by one.
var s = "[";
for (var i = 0; i < 1 << 16; i++) s += "\\u{" + (i * 3).toString(16) + "}";
new RegExp(s + "]", "iu");
"compiled";
