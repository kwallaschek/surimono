// A pattern nested 100,000 levels deep: the regex compiler recurses.
String(new RegExp("(".repeat(100000) + "a" + ")".repeat(100000)).test("a"));
