const s = 'function f(){' + 'a=1;'.repeat(1 << 24) + '}';
armDeadline(); (0, eval)(s);
