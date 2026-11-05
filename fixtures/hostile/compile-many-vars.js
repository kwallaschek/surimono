// Each top-level var searches the ones before it.
var s = "";
for (var i = 0; i < 1 << 17; i++) s += "var a" + i + ";";
(0, eval)(s);
"compiled";
