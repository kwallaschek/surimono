// A million forward references, each resolved by a scan of the pattern.
new RegExp("\\k<n>".repeat(1 << 20) + "(?<n>x)");
"compiled";
