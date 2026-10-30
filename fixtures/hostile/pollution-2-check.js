// Render 2 of 2: nothing from render 1 may be visible.
[
  typeof {}.polluted,
  [1, 2].map((x) => x).length,
  typeof JSON.parse("{}"),
  " x ".trim(),
  typeof globalThis.leak,
  typeof globalThis.ghost,
  Object.isFrozen(Object.prototype),
  String(Math.random).includes("[native code]"),
].join(",");
