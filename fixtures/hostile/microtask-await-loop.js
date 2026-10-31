// An async function that never stops awaiting: each await is a microtask.
(async () => {
  for (;;) await null;
})();
