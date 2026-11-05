const parts = [];
for (let i = 0; i < 1 << 22; i++) parts.push('a' + i);
const s = '[\\q{' + parts.join('|') + '}]';
armDeadline(); new RegExp(s, 'v');
