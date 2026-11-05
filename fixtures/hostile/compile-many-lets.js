// 1 MiB of source, quadratic: each name walks every variable in scope when the
// variables are resolved, after the parser is done.
var s = "function f(){";
for (var i = 0; i < 60000; i++) s += "let a" + i + "=a" + (i >> 1) + ";";
s += "}";
(0, eval)(s);
"compiled";
