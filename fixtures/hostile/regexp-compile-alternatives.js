// One regular expression of 2 million alternatives: no match runs.
new RegExp("x|".repeat(1 << 21));
"compiled";
