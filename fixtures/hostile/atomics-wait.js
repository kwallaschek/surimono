// Atomics.wait would block the thread in a futex, where no interrupt can
// reach it. In an agent that cannot suspend, Atomics.wait throws a TypeError
// (ECMA-262, DoWait: AgentCanSuspend() is false), as on a browser's main thread. If
// SharedArrayBuffer is missing altogether, that is a ReferenceError: also fine.
const i32 = new Int32Array(new SharedArrayBuffer(4));
Atomics.wait(i32, 0, 0);
"returned";
