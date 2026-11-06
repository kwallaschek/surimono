  // ------ CSSOM, as far as insertRule, cssRules and deleteRule (CSSOM §6) ------
  //
  // A sheet is read from its element's text (src/sheet.rs) when first asked for, and
  // lives here, in the heap, as a list of rule records. An edit also goes to Rust,
  // which writes the rules in the printed page in place of the text. A new text
  // makes a new sheet, as in a browser. No stylesheet applies here: nothing asks
  // these rules anything until layout.

  const styleByTag = protos[I_Document].getElementsByTagName;

  // An object with indexed properties and nothing else, for what has no handle:
  // the record has `count()` and `item(i)`. WebIDL §3.9, as `exotic`, read only.
  function indexed(s) {
    const h = create(null);
    const lookup = (t, k) => {
      const i = indexKey(k);
      if (i < 0) return undefined;
      const r = stateOf(t);
      return i < r.count() ? r.item(i) : undefined;
    };
    h.get = (t, k, r) => { const v = lookup(t, k); return v !== undefined ? v : rget(t, k, r); };
    h.has = (t, k) => lookup(t, k) !== undefined || rhas(t, k);
    h.getOwnPropertyDescriptor = (t, k) => {
      const v = lookup(t, k);
      if (v === undefined) return rdescriptor(t, k);
      const d = create(null);
      d.value = v; d.configurable = true; d.enumerable = true; d.writable = false;
      return d;
    };
    h.set = (t, k, v, r) => (indexKey(k) >= 0 ? false : rset(t, k, v, r));
    h.defineProperty = (t, k, d) => (indexKey(k) >= 0 ? false : rdefine(t, k, d));
    h.deleteProperty = (t, k) => (lookup(t, k) !== undefined ? false : rdelete(t, k));
    h.ownKeys = (t) => {
      const keys = create(null);
      const r = stateOf(t);
      let m = 0;
      const c = r.count();
      for (let i = 0; i < c; i++) keys[m++] = `${i}`;
      const own_ = rownKeys(t);
      for (let i = 0; i < own_.length; i++) keys[m++] = own_[i];
      keys.length = m;
      return keys;
    };
    s.w = new Brand(new ProxyCtor(s.w, h), s);
    return s;
  }
  defineProperty(protos[I_CSSRuleList], symbolIterator, { value: arrayValues, writable: true, configurable: true });
  defineProperty(protos[I_StyleSheetList], symbolIterator, { value: arrayValues, writable: true, configurable: true });

  for (const id of [I_CSSRuleList, I_StyleSheetList]) {
    op(id, 'item', function item(index) {
      const l = self(this, id);
      need(arguments.length, 1, 'item');
      const i = index >>> 0;
      return i < l.count() ? l.item(i) : null;
    });
  }

  function makeRule(sheet, kind, prelude, text) {
    const r = make(kind === 1 ? I_CSSStyleRule : I_CSSRule);
    r.kind = kind; r.prelude = prelude; r.text = text; r.sheet = sheet;
    return r;
  }
  // Fields, each after a NUL, three to a rule.
  function eachRule(joined, each) {
    let at = 1;
    while (at < joined.length) {
      const a = apply(indexOf, joined, ['\0', at]);
      const b = apply(indexOf, joined, ['\0', a + 1]);
      let end = apply(indexOf, joined, ['\0', b + 1]);
      if (end < 0) end = joined.length;
      each(+apply(slice, joined, [at, a]), apply(slice, joined, [a + 1, b]),
        apply(slice, joined, [b + 1, end]));
      at = end + 1;
    }
  }
  function newSheet(owner, text) {
    const s = make(I_CSSStyleSheet);
    s.owner = owner; s.source = text; s.rules = list();
    eachRule(n.cssRules(text), (kind, prelude, rule) => {
      add(s.rules, makeRule(s, kind, prelude, rule));
    });
    const l = make(I_CSSRuleList);
    l.sheet = s;
    l.count = () => s.rules.n;
    l.item = (i) => s.rules[i].w;
    s.list = indexed(l).w;
    return s;
  }
  // HTML §4.2.6: a `type` of anything but CSS has no sheet.
  function cssType(e) {
    const t = attrOf(e, 'type');
    return t === null || t === '' || asciiLower(t) === 'text/css';
  }
  // A sheet is its element's until the text next changes, even back to the same
  // text, and only while the element is in a tree: Rust counts the changes.
  const generation = (e) => call(OP_sheetGeneration, e.g, e.h);
  H.HTMLStyleElement_sheet = (e) => {
    if (!connected(e) || !cssType(e)) return null;
    const g = generation(e);
    if (e.sheet === undefined || e.sheetGeneration !== g) {
      e.sheet = newSheet(e, call(OP_sheetText, e.g, e.h));
      e.sheetGeneration = g;
    }
    return e.sheet.w;
  };
  H.HTMLLinkElement_sheet = () => null;
  H.StyleSheet_type = () => 'text/css';
  H.StyleSheet_href = () => null;
  H.StyleSheet_ownerNode = (s) => (current_(s) ? s.owner.w : null);
  H.StyleSheet_parentStyleSheet = () => null;
  H.StyleSheet_title = (s) => {
    const v = attrOf(s.owner, 'title');
    return v === null || v === '' ? null : v;
  };
  H.CSSStyleSheet_ownerRule = () => null;
  H.CSSStyleSheet_cssRules = (s) => s.list;
  H.CSSStyleSheet_rules = (s) => s.list;
  // A sheet its element has replaced still works, and is nothing to the page.
  function current_(s) {
    const e = s.owner;
    return e.sheet === s && e.sheetGeneration === generation(e) && connected(e);
  }
  // CSSOM "insert a CSS rule", without what needs a rule's meaning: `@import`
  // comes before other rules, and a statement can't follow a style rule.
  function insertRule(s, text, index) {
    const fields = n.cssRule(text);
    if (index > s.rules.n) indexError('the index');
    if (fields === undefined) throwDom('SyntaxError', 'the rule can\'t be parsed');
    eachRule(fields, (kind, prelude, rule) => {
      if (kind === 3) {
        for (let i = 0; i < index; i++) {
          if (s.rules[i].kind !== 3) hierarchy('an @import comes before other rules');
        }
      } else {
        if (kind === 10) {
          for (let i = 0; i < s.rules.n; i++) {
            if (s.rules[i].kind !== 3 && s.rules[i].kind !== 10) {
              throwDom('InvalidStateError', 'a sheet with rules takes no @namespace');
            }
          }
        }
        for (let i = index; i < s.rules.n; i++) {
          if (s.rules[i].kind === 3) hierarchy('an @import comes before other rules');
        }
      }
      if (current_(s)) call(OP_sheetInsert, s.owner.g, s.owner.h, index, rule);
      for (let i = s.rules.n; i > index; i--) s.rules[i] = s.rules[i - 1];
      s.rules[index] = makeRule(s, kind, prelude, rule);
      s.rules.n++;
    });
    return index;
  }
  function deleteRule(s, index) {
    if (index >= s.rules.n) indexError('the index');
    if (current_(s)) call(OP_sheetDelete, s.owner.g, s.owner.h, index);
    s.rules[index].sheet = null;
    for (let i = index + 1; i < s.rules.n; i++) s.rules[i - 1] = s.rules[i];
    delete s.rules[--s.rules.n];
  }
  H.CSSStyleSheet_insertRule = insertRule;
  H.CSSStyleSheet_deleteRule = deleteRule;
  H.CSSStyleSheet_removeRule = deleteRule;
  H.CSSStyleSheet_addRule = (s, selector, style, index) => {
    const at = index === undefined ? s.rules.n : index;
    insertRule(s, `${selector} { ${style} }`, at);
    return -1;
  };
  H.CSSRuleList_length = (l) => l.count();
  H.StyleSheetList_length = (l) => l.count();
  H.CSSRule_cssText = (r) => r.text;
  H.set_CSSRule_cssText = () => {};
  H.CSSRule_parentRule = () => null;
  H.CSSRule_parentStyleSheet = (r) => (r.sheet === null ? null : r.sheet.w);
  H.CSSRule_type = (r) => r.kind;
  H.CSSStyleRule_selectorText = (r) => r.prelude;
  // Only a selector with no rule's shape changes nothing: as for `insertRule`, what
  // the selector means isn't judged.
  H.set_CSSStyleRule_selectorText = (r, v) => {
    const s = r.sheet;
    if (s === null) return;
    const body = apply(slice, r.text, [r.prelude.length]);
    const fields = n.cssRule(`${v}${body}`);
    if (fields === undefined) return;
    eachRule(fields, (kind, prelude, rule) => {
      if (kind !== 1) return;
      let at = 0;
      while (s.rules[at] !== r) at++;
      if (current_(s)) {
        call(OP_sheetDelete, s.owner.g, s.owner.h, at);
        call(OP_sheetInsert, s.owner.g, s.owner.h, at, rule);
      }
      r.prelude = prelude; r.text = rule;
    });
  };
  H.Document_styleSheets = (d) => {
    if (d.styleSheets === undefined) {
      const l = make(I_StyleSheetList);
      const elements = apply(styleByTag, d.w, ['style']);
      const sheets = () => {
        const out = list();
        const m = size(elements);
        for (let i = 0; i < m; i++) {
          const e = stateOf(at(elements, i));
          if (e.m[I_HTMLStyleElement] !== true) continue;
          const sheet = H.HTMLStyleElement_sheet(e);
          if (sheet !== null) add(out, sheet);
        }
        return out;
      };
      l.count = () => sheets().n;
      l.item = (i) => sheets()[i];
      d.styleSheets = indexed(l).w;
    }
    return d.styleSheets;
  };
