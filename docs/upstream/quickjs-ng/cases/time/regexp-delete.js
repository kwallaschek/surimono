const s = 'a'.repeat(1 << 27);
armDeadline(); s.replace(/(?:x|y|z|w|v|u|t|a)/g, '');
