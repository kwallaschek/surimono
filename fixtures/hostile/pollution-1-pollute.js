// Render 1 of 2: pollute and tamper with everything shared. Render 2, in the
// same pooled runtime, must see none of it.
Object.prototype.polluted = "yes";
Array.prototype.map = function () {
  return "hijacked";
};
JSON.parse = () => "hijacked";
String.prototype.trim = () => "hijacked";
globalThis.leak = "leaked";
Math.random = () => 0.5;
if (typeof FinalizationRegistry === "function") {
  new FinalizationRegistry(() => {
    globalThis.ghost = "boo";
  }).register({}, 0);
}
Object.freeze(Object.prototype);
"polluted";
