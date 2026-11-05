// The same compile through the Function constructor.
var s = "";
for (var i = 0; i < 60000; i++) s += "let a" + i + "=a" + (i >> 1) + ";";
new Function(s);
"compiled";
