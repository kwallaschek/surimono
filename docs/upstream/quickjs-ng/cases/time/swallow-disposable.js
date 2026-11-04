armDeadline(); for (;;) { try { const s = new DisposableStack(); s.defer(() => { for (;;) {} }); s.defer(() => { for (;;) {} }); s.dispose() } catch (e) {} }
