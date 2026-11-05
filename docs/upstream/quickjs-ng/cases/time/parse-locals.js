let s = 'function f(){';
for (let i = 0; i < 60000; i++) s += `let a${i}=a${i >> 1};`;
s += '}';
armDeadline(); (0, eval)(s);
