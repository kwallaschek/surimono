// The same compile inside a try: the time limit stays uncatchable when it
// arrives during a compile, not while bytecode runs.
var s = "function f(){";
for (var i = 0; i < 60000; i++) s += "let a" + i + "=a" + (i >> 1) + ";";
s += "}";
try { (0, eval)(s); } catch (e) {}
"caught";
