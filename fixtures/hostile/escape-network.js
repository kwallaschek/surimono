// Network APIs: absent, or present but refusing (phase 2 may add fetch with
// a caller-supplied loader that is off by default). Never a response.
(async () => {
  const results = [];
  if (typeof fetch === "function") {
    try {
      await fetch("https://example.com/");
      results.push("fetch ESCAPED");
    } catch (e) {
      results.push("fetch denied");
    }
  }
  for (const name of ["XMLHttpRequest", "WebSocket", "EventSource", "Worker", "SharedWorker"]) {
    if (typeof globalThis[name] === "function") {
      try {
        if (name === "XMLHttpRequest") {
          const x = new XMLHttpRequest();
          x.open("GET", "https://example.com/", false);
          x.send();
          results.push(name + " ESCAPED");
        } else {
          new globalThis[name]("https://example.com/");
          results.push(name + " constructed");
        }
      } catch (e) {
        results.push(name + " denied");
      }
    }
  }
  return results.some((r) => /ESCAPED|constructed/.test(r)) ? results.join(",") : "denied";
})();
