// Catastrophic backtracking: (a+)+$ against "aaaa...b" takes 2^n steps.
// The regex engine must check the time limit while it backtracks.
/^(a+)+$/.test("a".repeat(40) + "!");
