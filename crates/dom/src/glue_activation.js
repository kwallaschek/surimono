  // Activation behaviour: what a click does after its listeners (DOM §2.9,
  // HTML §6.6 "click", §4.6.4 hyperlinks, §4.10 forms, §4.11.1 details). Part
  // of the function glue_head.js opens. A page is printed, not used, so
  // nothing here leaves the document: submitting a form fires its events and
  // stops where it would navigate; a link only runs a `javascript:` URL.

  function attrOf(s, name) { return call(OP_getAttribute, s.g, s.h, name); }
  function parentState(s) {
    const p = wrap(s.g, call(OP_parentNode, s.g, s.h));
    return p === null ? null : stateOf(p);
  }
  function isHtml(s) { return s.m[I_HTMLElement] === true; }
  function localOf(s) { return isHtml(s) ? call(OP_localName, s.g, s.h) : ''; }
  function connected(s) { return call(OP_isConnected, s.g, s.h); }

  function hasActivation(s) {
    switch (localOf(s)) {
      case 'a': case 'area': case 'button': case 'input': case 'label': case 'summary':
        return true;
      default:
        return false;
    }
  }

  // HTML §4.10.18.5 "disabled", for a control in a fieldset too: a disabled
  // fieldset's first legend keeps its contents enabled.
  function isDisabled(s) {
    if (attrOf(s, 'disabled') !== null) return true;
    let child = s;
    for (let at = parentState(s); at !== null; child = at, at = parentState(at)) {
      if (localOf(at) !== 'fieldset' || attrOf(at, 'disabled') === null) continue;
      if (!isFirstLegend(at, child)) return true;
    }
    return false;
  }
  function isFirstLegend(fieldset, child) {
    if (localOf(child) !== 'legend') return false;
    let c = wrap(fieldset.g, call(OP_firstElementChild, fieldset.g, fieldset.h));
    while (c !== null) {
      const cs = stateOf(c);
      if (localOf(cs) === 'legend') return cs === child;
      c = wrap(cs.g, call(OP_nextElementSibling, cs.g, cs.h));
    }
    return false;
  }
  function isControl(s) {
    switch (localOf(s)) {
      case 'button': case 'input': case 'select': case 'textarea': case 'fieldset':
        return true;
      default:
        return false;
    }
  }

  // HTML §4.10.17.3 "form owner": the `form` attribute's element if connected
  // and the tree has one, otherwise the nearest ancestor.
  function formOwner(s) {
    const id = attrOf(s, 'form');
    if (id !== null && connected(s)) {
      const f = wrap(s.g, call(OP_getElementById, s.g, root(), id));
      if (f === null) return null;
      const fs = stateOf(f);
      return localOf(fs) === 'form' ? fs : null;
    }
    for (let at = parentState(s); at !== null; at = parentState(at)) {
      if (localOf(at) === 'form') return at;
    }
    return null;
  }
  const formOf = (s) => {
    const f = formOwner(s);
    return f === null ? null : f.w;
  };
  H.HTMLInputElement_form = formOf;
  H.HTMLButtonElement_form = formOf;
  H.HTMLSelectElement_form = formOf;
  H.HTMLTextAreaElement_form = formOf;
  H.HTMLFieldSetElement_form = formOf;
  H.HTMLOutputElement_form = formOf;

  H.HTMLButtonElement_type = (s) => {
    const t = asciiLower(`${attrOf(s, 'type')}`);
    return t === 'reset' || t === 'button' ? t : 'submit';
  };
  H.set_HTMLButtonElement_type = (s, v) => { call(OP_setAttribute, s.g, s.h, 'type', v); };

  H.HTMLInputElement_indeterminate = (s) => s.indeterminate === true;
  H.set_HTMLInputElement_indeterminate = (s, v) => { s.indeterminate = v; };

  // HTML §4.10.4: the labeled control.
  function labeledControl(s) {
    const c = wrap(s.g, call(OP_labeledControl, s.g, s.h));
    return c === null ? null : stateOf(c);
  }
  H.HTMLLabelElement_form = (s) => {
    const c = labeledControl(s);
    return c === null ? null : formOf(c);
  };
  H.HTMLLabelElement_control = (s) => {
    const c = labeledControl(s);
    return c === null ? null : c.w;
  };

  // HTML §3.2.5.2.7 "interactive content".
  function interactive(s) {
    switch (localOf(s)) {
      case 'button': case 'details': case 'embed': case 'iframe': case 'label': case 'select':
      case 'textarea':
        return true;
      case 'a': return attrOf(s, 'href') !== null;
      case 'audio': case 'video': return attrOf(s, 'controls') !== null;
      case 'img': return attrOf(s, 'usemap') !== null;
      case 'input': return call(OP_type, s.g, s.h) !== 'hidden';
      default: return false;
    }
  }

  // HTML §6.6: a synthetic pointer event, not trusted when `click()` fires it.
  function syntheticClick(s) {
    const e = make(I_PointerEvent);
    initEvent(e, 'click', true, true);
    e.composed = true;
    e.d = eventInits[I_PointerEvent]();
    e.d.view = G;
    return dispatch(s, e);
  }
  H.HTMLElement_click = (s) => {
    if (isControl(s) && isDisabled(s)) return;
    if (s.clicking === true) return;
    s.clicking = true;
    try {
      syntheticClick(s);
    } finally {
      s.clicking = false;
    }
  };

  // Input's legacy-pre-activation behaviour; the result is for undoing it.
  function preActivate(s) {
    if (localOf(s) !== 'input') return null;
    const t = call(OP_type, s.g, s.h);
    if (t !== 'checkbox' && t !== 'radio') return null;
    const saved = create(null);
    saved.checked = call(OP_checked, s.g, s.h);
    saved.indeterminate = s.indeterminate === true;
    saved.other = 0;
    if (t === 'checkbox') {
      call(OP_set_checked, s.g, s.h, !saved.checked);
    } else {
      saved.other = call(OP_radioCheck, s.g, s.h);
    }
    s.indeterminate = false;
    return saved;
  }
  function cancelActivation(s, saved) {
    if (saved === null) return;
    call(OP_set_checked, s.g, s.h, saved.checked);
    s.indeterminate = saved.indeterminate;
    if (saved.other !== 0) call(OP_set_checked, s.g, saved.other, true);
  }

  function fireControl(s) {
    fire(s, 'input', true, false, true);
    fire(s, 'change', true);
  }

  // HTML §4.10.21.3 "submit", as far as this side of navigating. No script
  // can see the rest; interactive validation isn't there yet (`required` and
  // the like don't stop it).
  function submitForm(form, submitter) {
    if (!connected(form) || form.submitting === true) return;
    form.submitting = true;
    const e = make(I_SubmitEvent);
    initEvent(e, 'submit', true, true);
    e.isTrusted = true;
    e.d = eventInits[I_SubmitEvent]();
    e.d.submitter = submitter === null ? null : submitter.w;
    try {
      dispatch(form, e);
    } finally {
      form.submitting = false;
    }
  }
  function resetForm(form) {
    if (form.resetting === true) return;
    form.resetting = true;
    try {
      fire(form, 'reset', true, true);
    } finally {
      form.resetting = false;
    }
  }
  H.HTMLFormElement_submit = () => {};
  H.HTMLFormElement_reset = resetForm;
  H.HTMLFormElement_requestSubmit = (form, submitter) => {
    if (submitter !== null) {
      const local = localOf(submitter);
      const t = local === 'button' ? H.HTMLButtonElement_type(submitter)
        : local === 'input' ? call(OP_type, submitter.g, submitter.h) : '';
      if (t !== 'submit' && t !== 'image') throw new TE('requestSubmit: not a submit button');
      if (formOwner(submitter) !== form) {
        throwDom('NotFoundError', 'requestSubmit: the submitter belongs to another form');
      }
    }
    submitForm(form, submitter);
  };

  // HTML §4.11.1: the details element's toggle event comes in a task, and
  // tasks waiting for one element coalesce.
  function toggleDetails(details) {
    const open = attrOf(details, 'open') !== null;
    if (open) call(OP_removeAttribute, details.g, details.h, 'open');
    else call(OP_setAttribute, details.g, details.h, 'open', '');
    const old = open ? 'open' : 'closed';
    if (details.toggle !== undefined) return;
    details.toggle = old;
    task(() => {
      const e = make(I_ToggleEvent);
      initEvent(e, 'toggle', false, false);
      e.isTrusted = true;
      e.alone = true;
      e.d = eventInits[I_ToggleEvent]();
      e.d.oldState = details.toggle;
      e.d.newState = attrOf(details, 'open') !== null ? 'open' : 'closed';
      details.toggle = undefined;
      dispatch(details, e);
    }, 0);
  }
  function summaryOf(details) {
    let c = wrap(details.g, call(OP_firstElementChild, details.g, details.h));
    while (c !== null) {
      const cs = stateOf(c);
      if (localOf(cs) === 'summary') return cs;
      c = wrap(cs.g, call(OP_nextElementSibling, cs.g, cs.h));
    }
    return null;
  }

  // HTML §4.6.4 "follow the hyperlink": `navigate` does what doesn't leave
  // the page. A link acts on its own document, if that has a browsing context,
  // and with a target only on one that stays in it (§7.3.1 "the rules for
  // choosing a navigable"; a `<base target>` is a gap). A `download` link has
  // nowhere to save to, and only an `a` may follow from outside a document
  // ("cannot navigate").
  function followLink(s) {
    const href = attrOf(s, 'href');
    if (href === null || attrOf(s, 'download') !== null) return;
    if (localOf(s) !== 'a' && !connected(s)) return;
    const d = stateOf(wrap(s.g, call(OP_ownerDocument, s.g, s.h)));
    const page = isPage(d);
    if (!page && !call(OP_isFrameDocument, d.g, d.h)) return;
    const target = asciiLower(attrOf(s, 'target') || '');
    const stays = target === '' || target === '_self' ||
      (page && (target === '_parent' || target === '_top'));
    if (stays) navigate(d, href, false, false);
  }
  function percentDecode(x) {
    try {
      return decodeURI_(x);
    } catch (e) {
      if (n.limit(e)) throw e;
      return x;
    }
  }

  function runActivation(s, event) {
    switch (localOf(s)) {
      case 'a': case 'area':
        followLink(s);
        break;
      case 'input': {
        const t = call(OP_type, s.g, s.h);
        if (t === 'checkbox' || t === 'radio') {
          if (connected(s)) fireControl(s);
          break;
        }
        if (t !== 'submit' && t !== 'image' && t !== 'reset') break;
        if (isDisabled(s)) break;
        const form = formOwner(s);
        if (form === null || !connected(s)) break;
        if (t === 'reset') resetForm(form);
        else submitForm(form, s);
        break;
      }
      case 'button': {
        if (isDisabled(s)) break;
        const form = formOwner(s);
        if (form === null) break;
        const t = H.HTMLButtonElement_type(s);
        if (t === 'submit') submitForm(form, s);
        else if (t === 'reset') resetForm(form);
        break;
      }
      case 'label': {
        // Nothing for a click on what is interactive already.
        const target = stateOf(event.target);
        for (let at = target; at !== null && at !== s; at = parentState(at)) {
          if (interactive(at)) return;
        }
        const c = labeledControl(s);
        if (c === null || isDisabled(c)) break;
        H.HTMLElement_click(c);
        break;
      }
      case 'summary': {
        const p = parentState(s);
        if (p !== null && localOf(p) === 'details' && summaryOf(p) === s) toggleDetails(p);
        break;
      }
      default:
    }
  }
