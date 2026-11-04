const o = []; for (let i = 0; i < 20000; i++) o.push({ i, s: 'x' + i });
armDeadline(); for (;;) JSON.stringify(o);
