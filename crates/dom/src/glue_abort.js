  // AbortController and AbortSignal (DOM §3). Part of the function
  // glue_head.js opens. `s.signals`/`s.sources` keep a signal's dependents and
  // sources strongly, under the memory limit; the spec's weak links make a
  // difference only to memory, and everything here ends with the render.

  function newSignal() {
    const s = make(I_AbortSignal);
    s.aborted = false; s.reason = undefined; s.algorithms = list();
    s.dependents = list(); s.sources = list(); s.composite = false;
    return s;
  }
  function abortReason(reason) {
    return reason === undefined ? new DOMExceptionCtor('This operation was aborted', 'AbortError') : reason;
  }
  // §3.2 "signal abort".
  function signalAbort(s, reason) {
    if (s.aborted) return;
    s.aborted = true;
    s.reason = abortReason(reason);
    const dependents = list();
    for (let i = 0; i < s.dependents.n; i++) {
      const d = s.dependents[i];
      if (!d.aborted) {
        d.aborted = true;
        d.reason = s.reason;
        add(dependents, d);
      }
    }
    runAbortSteps(s);
    for (let i = 0; i < dependents.n; i++) runAbortSteps(dependents[i]);
  }
  function runAbortSteps(s) {
    const algorithms = s.algorithms;
    s.algorithms = list();
    for (let i = 0; i < algorithms.n; i++) algorithms[i]();
    fire(s, 'abort');
  }
  H.new_AbortController = (target) => {
    const s = make(I_AbortController, target);
    s.signal = newSignal();
    return s.w;
  };
  H.AbortController_signal = (s) => s.signal.w;
  H.AbortController_abort = (s, reason) => { signalAbort(s.signal, reason); };
  H.AbortSignal_aborted = (s) => s.aborted;
  H.AbortSignal_reason = (s) => s.reason;
  H.AbortSignal_throwIfAborted = (s) => { if (s.aborted) throw s.reason; };
  H.AbortSignal_abort = (reason) => {
    const s = newSignal();
    s.aborted = true;
    s.reason = abortReason(reason);
    return s.w;
  };
  // [EnforceRange] unsigned long long, on the virtual clock.
  H.AbortSignal_timeout = (milliseconds) => {
    const v = +milliseconds;
    if (v - v !== 0 || v < 0) throw new TE('timeout: the value is out of range');
    const s = newSignal();
    setTimeoutReal(() => {
      signalAbort(s, new DOMExceptionCtor('The operation timed out', 'TimeoutError'));
    }, v);
    return s.w;
  };
  // §3.2.3 "create a dependent abort signal".
  H.AbortSignal_any = (signals) => {
    const given = seq(signals, (e) => stateOf(inst(e, I_AbortSignal)));
    const r = newSignal();
    r.composite = true;
    const n_ = given.length;
    for (let i = 0; i < n_; i++) {
      const g = given[i];
      if (g.aborted) { r.aborted = true; r.reason = g.reason; return r.w; }
    }
    for (let i = 0; i < n_; i++) {
      const g = given[i];
      if (!g.composite) {
        add(r.sources, g);
        add(g.dependents, r);
      } else {
        for (let k = 0; k < g.sources.n; k++) {
          add(r.sources, g.sources[k]);
          add(g.sources[k].dependents, r);
        }
      }
    }
    return r.w;
  };
