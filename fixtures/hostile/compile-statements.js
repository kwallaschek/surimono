// A function nothing calls, 16 MiB of statements: all the cost is the compile.
(0, eval)("function f(){" + "a=1;".repeat(1 << 22) + "}");
"compiled";
