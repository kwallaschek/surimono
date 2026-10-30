// Control case: a promise result is awaited (Vega's toSVG is async).
Promise.resolve("async").then((v) => v + ":ok");
