// A direct eval can use every variable of the enclosing functions, so each one
// goes through each function in between, before the first pass over the code.
var s = "";
for (var i = 0; i < 1 << 15; i++) s += "var a" + i + ";";
(0, eval)("function f(){" + s + "return " + "()=>".repeat(16) + "eval(\"\")}");
"compiled";
