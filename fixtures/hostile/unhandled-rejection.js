// An unhandled rejection does not fail the render, as in browsers, where it
// only logs. The result is the completion value.
Promise.reject(new Error("ignored"));
"ok";
