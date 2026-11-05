// 8 million numbers: a long native parse with no script in it.
JSON.parse("[" + "1,".repeat(1 << 23) + "1]");
"parsed";
