const s = 'function f(a){' + ('if(a){' + 'a();'.repeat(12) + '}').repeat(32000) + '}';
armDeadline(); (0, eval)(s);
