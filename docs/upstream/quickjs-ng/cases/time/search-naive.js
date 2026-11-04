const s = 'a'.repeat(1 << 20), p = 'a'.repeat(1 << 12) + 'b';
armDeadline(); s.indexOf(p);
