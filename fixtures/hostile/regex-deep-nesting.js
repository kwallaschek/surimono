// A pattern nested 100,000 levels deep: the regex compiler recurses.
// Non-capturing groups, so the engine's limit on capture groups (255 in
// QuickJS) does not stop it before the recursion does.
String(new RegExp("(?:".repeat(100000) + "a" + ")".repeat(100000)).test("a"));
