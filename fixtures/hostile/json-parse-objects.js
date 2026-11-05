// 2 million small objects.
JSON.parse("[" + "{\"a\":1},".repeat(1 << 21) + "1]");
"parsed";
