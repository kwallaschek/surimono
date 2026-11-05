const s = '[' + '{"a":1},'.repeat(1 << 23) + '1]';
armDeadline(); JSON.parse(s);
