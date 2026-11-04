const t = new Int32Array(3e7); for (let i = 0; i < t.length; i++) t[i] = Math.imul(i, 2654435761);
armDeadline(); t.sort();
