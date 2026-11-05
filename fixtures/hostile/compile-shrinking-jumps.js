// Forward jumps that need the long form until the code between them is
// optimized. Each one shrunk moves the rest of the bytecode: 880 KiB, quadratic,
// all of it after the per-instruction passes.
var block = "if(a){" + "a();".repeat(12) + "}";
(0, eval)("function f(a){" + block.repeat(16000) + "}");
"compiled";
