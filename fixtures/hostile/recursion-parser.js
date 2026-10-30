// Native recursion in the JavaScript parser: 100,000 nested parentheses.
const n = 100000;
String(eval("(".repeat(n) + "1" + ")".repeat(n)));
