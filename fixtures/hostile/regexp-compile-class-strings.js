// \q{} strings in a class: one pass per list operation, all inside one class item.
var s = "[\\q{";
for (var i = 0; i < 1 << 20; i++) s += "a" + i + "|";
new RegExp(s + "b}]", "iv");
"compiled";
