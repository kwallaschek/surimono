const s = 'a'.repeat(1 << 27);
armDeadline(); s.replaceAll('a', '');
