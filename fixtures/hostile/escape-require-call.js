// Calling require must throw (ReferenceError when it is not defined).
require("fs").readFileSync("/etc/passwd", "utf8");
