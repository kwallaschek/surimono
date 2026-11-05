let s = 'function f(){';
for (let i = 0; i < 32768; i++) s += `var a${i};`;
s += 'return ' + '()=>'.repeat(16) + 'eval("")}';
armDeadline(); (0, eval)(s);
