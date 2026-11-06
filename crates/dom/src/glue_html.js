  // ------ HTML's own members over the tree ------
  //
  // Tables (HTML §4.9), selects (§4.10.7), forms (§4.10.3), hyperlinks' URL
  // decomposition (§4.6.3), focus (§6.6) and the document's cookie. What only
  // moves nodes is here, through the DOM's own members, captured now; the lists
  // are Rust's (src/html.rs).

  const XHTML = 'http://www.w3.org/1999/xhtml';
  const nodeProto = protos[I_Node], elementProto = protos[I_Element];
  const appendChild_ = nodeProto.appendChild, insertBefore_ = nodeProto.insertBefore;
  const removeChild_ = nodeProto.removeChild;
  const createElementNS_ = protos[I_Document].createElementNS;
  const ownerDocument_ = rdescriptor(nodeProto, 'ownerDocument').get;
  const parentNode_ = rdescriptor(nodeProto, 'parentNode').get;
  const textContent_ = rdescriptor(nodeProto, 'textContent');
  const firstChild_ = rdescriptor(nodeProto, 'firstChild').get;
  const nextSibling_ = rdescriptor(nodeProto, 'nextSibling').get;
  const nodeValue_ = rdescriptor(nodeProto, 'nodeValue').get;
  const nodeType_ = rdescriptor(nodeProto, 'nodeType').get;
  const firstElementChild_ = rdescriptor(elementProto, 'firstElementChild').get;
  const nextElementSibling_ = rdescriptor(elementProto, 'nextElementSibling').get;
  const namespaceURI_ = rdescriptor(elementProto, 'namespaceURI').get;
  const localName_ = rdescriptor(elementProto, 'localName').get;
  const collectionLength = rdescriptor(protos[I_HTMLCollection], 'length').get;
  const collectionItem = protos[I_HTMLCollection].item;
  const collectionNamed = protos[I_HTMLCollection].namedItem;

  const isElement = (w, local) => apply(localName_, w, []) === local &&
    apply(namespaceURI_, w, []) === XHTML;
  function firstChild(parent, local) {
    for (let c = apply(firstElementChild_, parent, []); c !== null;
      c = apply(nextElementSibling_, c, [])) {
      if (isElement(c, local)) return c;
    }
    return null;
  }
  function make_(parent, local) {
    return apply(createElementNS_, apply(ownerDocument_, parent, []), [XHTML, local]);
  }
  const insert = (parent, node, before) => apply(insertBefore_, parent, [node, before]);
  const remove = (parent, node) => apply(removeChild_, parent, [node]);
  const htmlList = (s, kind) => wrap(s.g, call(OP_htmlList, s.g, s.h, kind));
  const size = (list) => apply(collectionLength, list, []);
  const at = (list, i) => apply(collectionItem, list, [i]);
  function indexError(what) {
    throwDom('IndexSizeError', `${what} is out of range`);
  }
  function hierarchy(what) {
    throwDom('HierarchyRequestError', `${what}`);
  }

  // ------ Tables ------

  const TABLE_ROWS = 0, TABLE_BODIES = 1, SECTION_ROWS = 2, ROW_CELLS = 3;
  const OPTIONS = 4, SELECTED = 5, CONTROLS = 6;

  H.HTMLTableElement_caption = (s) => firstChild(s.w, 'caption');
  H.HTMLTableElement_deleteCaption = (s) => {
    const c = firstChild(s.w, 'caption');
    if (c !== null) remove(s.w, c);
  };
  H.set_HTMLTableElement_caption = (s, v) => {
    H.HTMLTableElement_deleteCaption(s);
    if (v !== null) insert(s.w, v.w, apply(firstChild_, s.w, []));
  };
  H.HTMLTableElement_createCaption = (s) => {
    let c = firstChild(s.w, 'caption');
    if (c === null) {
      c = make_(s.w, 'caption');
      insert(s.w, c, apply(firstChild_, s.w, []));
    }
    return c;
  };
  // The first element child after any `caption` and `colgroup`.
  function afterColumns(table) {
    let c = apply(firstElementChild_, table, []);
    while (c !== null && (isElement(c, 'caption') || isElement(c, 'colgroup'))) {
      c = apply(nextElementSibling_, c, []);
    }
    return c;
  }
  function setSection(s, v, local, place) {
    if (v !== null && !isElement(v.w, local)) hierarchy(`the value is not a ${local}`);
    const old = firstChild(s.w, local);
    if (old !== null) remove(s.w, old);
    if (v !== null) place(v.w);
  }
  H.HTMLTableElement_tHead = (s) => firstChild(s.w, 'thead');
  H.set_HTMLTableElement_tHead = (s, v) => {
    setSection(s, v, 'thead', (w) => { insert(s.w, w, afterColumns(s.w)); });
  };
  H.HTMLTableElement_createTHead = (s) => {
    let c = firstChild(s.w, 'thead');
    if (c === null) {
      c = make_(s.w, 'thead');
      insert(s.w, c, afterColumns(s.w));
    }
    return c;
  };
  H.HTMLTableElement_deleteTHead = (s) => { setSection(s, null, 'thead'); };
  H.HTMLTableElement_tFoot = (s) => firstChild(s.w, 'tfoot');
  H.set_HTMLTableElement_tFoot = (s, v) => {
    setSection(s, v, 'tfoot', (w) => { apply(appendChild_, s.w, [w]); });
  };
  H.HTMLTableElement_createTFoot = (s) => {
    let c = firstChild(s.w, 'tfoot');
    if (c === null) {
      c = make_(s.w, 'tfoot');
      apply(appendChild_, s.w, [c]);
    }
    return c;
  };
  H.HTMLTableElement_deleteTFoot = (s) => { setSection(s, null, 'tfoot'); };
  H.HTMLTableElement_tBodies = (s) => htmlList(s, TABLE_BODIES);
  H.HTMLTableElement_createTBody = (s) => {
    const body = make_(s.w, 'tbody');
    const bodies = htmlList(s, TABLE_BODIES);
    const m = size(bodies);
    const last = m === 0 ? null : apply(nextElementSibling_, at(bodies, m - 1), []);
    insert(s.w, body, m === 0 ? null : last);
    return body;
  };
  H.HTMLTableElement_rows = (s) => htmlList(s, TABLE_ROWS);
  H.HTMLTableElement_insertRow = (s, index) => {
    const rows = htmlList(s, TABLE_ROWS);
    const m = size(rows);
    if (index < -1 || index > m) indexError('the index');
    const row = make_(s.w, 'tr');
    if (m === 0) {
      const bodies = htmlList(s, TABLE_BODIES);
      const b = size(bodies);
      if (b === 0) {
        const body = make_(s.w, 'tbody');
        apply(appendChild_, body, [row]);
        apply(appendChild_, s.w, [body]);
      } else {
        apply(appendChild_, at(bodies, b - 1), [row]);
      }
    } else if (index === -1 || index === m) {
      apply(appendChild_, apply(parentNode_, at(rows, m - 1), []), [row]);
    } else {
      const next = at(rows, index);
      insert(apply(parentNode_, next, []), row, next);
    }
    return row;
  };
  // Deleting from a list: -1 is the last, and an empty list has none to delete.
  function deleteAt(list, index, parent) {
    const m = size(list);
    if (index < -1 || index >= m) indexError('the index');
    if (index === -1 && m === 0) return;
    const node = at(list, index === -1 ? m - 1 : index);
    remove(parent === undefined ? apply(parentNode_, node, []) : parent, node);
  }
  H.HTMLTableElement_deleteRow = (s, index) => {
    const rows = htmlList(s, TABLE_ROWS);
    if (index === -1 && size(rows) === 0) return;
    deleteAt(rows, index);
  };
  H.HTMLTableSectionElement_rows = (s) => htmlList(s, SECTION_ROWS);
  H.HTMLTableSectionElement_insertRow = (s, index) => {
    const rows = htmlList(s, SECTION_ROWS);
    const m = size(rows);
    if (index < -1 || index > m) indexError('the index');
    const row = make_(s.w, 'tr');
    insert(s.w, row, index === -1 || index === m ? null : at(rows, index));
    return row;
  };
  H.HTMLTableSectionElement_deleteRow = (s, index) => { deleteAt(htmlList(s, SECTION_ROWS), index, s.w); };
  function indexIn(list, w) {
    const m = size(list);
    for (let i = 0; i < m; i++) if (at(list, i) === w) return i;
    return -1;
  }
  // The table a row belongs to: its parent, or that parent's if it is a section.
  function tableOf(row) {
    const p = apply(parentNode_, row, []);
    if (p === null || stateOf(p).m[I_Element] !== true) return null;
    if (isElement(p, 'table')) return p;
    if (isElement(p, 'tbody') || isElement(p, 'thead') || isElement(p, 'tfoot')) {
      const t = apply(parentNode_, p, []);
      if (t !== null && stateOf(t).m[I_Element] === true && isElement(t, 'table')) return t;
    }
    return null;
  }
  H.HTMLTableRowElement_rowIndex = (s) => {
    const t = tableOf(s.w);
    return t === null ? -1 : indexIn(htmlList(stateOf(t), TABLE_ROWS), s.w);
  };
  H.HTMLTableRowElement_sectionRowIndex = (s) => {
    const p = apply(parentNode_, s.w, []);
    if (p === null) return -1;
    const ps = stateOf(p);
    if (ps.m[I_Element] !== true) return -1;
    const parentIsTable = isElement(p, 'table');
    if (!parentIsTable && !isElement(p, 'tbody') && !isElement(p, 'thead') && !isElement(p, 'tfoot')) {
      return -1;
    }
    return indexIn(htmlList(ps, parentIsTable ? TABLE_ROWS : SECTION_ROWS), s.w);
  };
  H.HTMLTableRowElement_cells = (s) => htmlList(s, ROW_CELLS);
  H.HTMLTableRowElement_insertCell = (s, index) => {
    const cells = htmlList(s, ROW_CELLS);
    const m = size(cells);
    if (index < -1 || index > m) indexError('the index');
    const cell = make_(s.w, 'td');
    insert(s.w, cell, index === -1 || index === m ? null : at(cells, index));
    return cell;
  };
  H.HTMLTableRowElement_deleteCell = (s, index) => { deleteAt(htmlList(s, ROW_CELLS), index, s.w); };
  H.HTMLTableCellElement_cellIndex = (s) => {
    const p = apply(parentNode_, s.w, []);
    if (p === null || stateOf(p).m[I_Element] !== true || !isElement(p, 'tr')) return -1;
    return indexIn(htmlList(stateOf(p), ROW_CELLS), s.w);
  };
  // HTML §2.6.1, an `unsigned long` limited to a range, clamped for these two.
  function spanOf(s, name, low, high) {
    const v = attrOf(s, name);
    const x = v === null ? NaN : parseInt_(v, 10);
    if (!(x >= 0)) return 1;
    return x < low ? low : x > high ? high : x;
  }
  const parseInt_ = parseInt;
  H.HTMLTableCellElement_colSpan = (s) => spanOf(s, 'colspan', 1, 1000);
  H.HTMLTableCellElement_rowSpan = (s) => spanOf(s, 'rowspan', 0, 65534);
  H.set_HTMLTableCellElement_colSpan = (s, v) => { call(OP_setAttribute, s.g, s.h, 'colspan', `${v}`); };
  H.set_HTMLTableCellElement_rowSpan = (s, v) => { call(OP_setAttribute, s.g, s.h, 'rowspan', `${v}`); };
  // §2.6.1 an enumerated attribute: the keywords, and nothing for anything else.
  H.HTMLTableCellElement_scope = (s) => {
    const v = asciiLower(reflectStr(s, 'scope'));
    return v === 'row' || v === 'col' || v === 'rowgroup' || v === 'colgroup' ? v : '';
  };
  H.set_HTMLTableCellElement_scope = (s, v) => { setReflectStr(s, 'scope', v); };

  // ------ Selects and options ------

  const selectOf = (option) => {
    let p = apply(parentNode_, option, []);
    if (p !== null && stateOf(p).m[I_Element] === true && isElement(p, 'optgroup')) {
      p = apply(parentNode_, p, []);
    }
    return p !== null && stateOf(p).m[I_Element] === true && isElement(p, 'select') ? stateOf(p) : null;
  };
  H.HTMLSelectElement_form = (s) => formOf(s);
  H.HTMLSelectElement_type = (s) => (attrOf(s, 'multiple') !== null ? 'select-multiple' : 'select-one');
  H.HTMLSelectElement_options = (s) => htmlList(s, OPTIONS);
  H.HTMLSelectElement_selectedOptions = (s) => htmlList(s, SELECTED);
  H.HTMLSelectElement_size = (s) => {
    const v = attrOf(s, 'size');
    const x = v === null ? NaN : parseInt_(v, 10);
    return x >= 0 && x <= 2147483647 ? x : 0;
  };
  H.set_HTMLSelectElement_size = (s, v) => { call(OP_setAttribute, s.g, s.h, 'size', `${v}`); };
  H.HTMLSelectElement_length = (s) => size(htmlList(s, OPTIONS));
  H.set_HTMLSelectElement_length = (s, v) => { setOptionCount(s, v); };
  // Not in the generator's table: `item` is a getter, and that would make `exotic`
  // ask Rust. So `select[0]` doesn't exist.
  op(I_HTMLSelectElement, 'item', function item(index) {
    const s = self(this, I_HTMLSelectElement);
    need(arguments.length, 1, 'item');
    return at(htmlList(s, OPTIONS), index >>> 0);
  });
  op(I_HTMLSelectElement, 'namedItem', function namedItem(name) {
    const s = self(this, I_HTMLSelectElement);
    need(arguments.length, 1, 'namedItem');
    return apply(collectionNamed, htmlList(s, OPTIONS), [`${name}`]);
  });
  // HTMLOptionsCollection's `length`: empty options to add, the last to remove.
  // HTML caps what one write can add, at 100,000.
  function setOptionCount(s, v) {
    const list = htmlList(s, OPTIONS);
    let m = size(list);
    if (v > 100000) return;
    while (m > v) {
      m--;
      const o = at(list, m);
      remove(apply(parentNode_, o, []), o);
    }
    for (; m < v; m++) apply(appendChild_, s.w, [make_(s.w, 'option')]);
  }
  function addOption(s, element, before) {
    let at_ = null;
    if (typeof before === 'number') {
      at_ = at(htmlList(s, OPTIONS), before);
    } else if (before !== null) {
      at_ = before;
    }
    if (at_ === null) {
      apply(appendChild_, s.w, [element]);
    } else {
      const p = apply(parentNode_, at_, []);
      if (p === null || !apply(protos[I_Node].contains, s.w, [p])) {
        throwDom('NotFoundError', 'the reference node is not in the select');
      }
      insert(p, element, at_);
    }
  }
  function beforeArgument(x) {
    if (x === null || x === undefined) return null;
    const b = stateOf(x);
    if (b !== null && b.m[I_HTMLElement] === true) return x;
    return x | 0;
  }
  function selectAdd(s, element, before) {
    const e = stateOf(element);
    if (e === null || (e.m[I_HTMLOptionElement] !== true && e.m[I_HTMLOptGroupElement] !== true)) {
      throw new TE('add: not an option or optgroup');
    }
    addOption(s, element, beforeArgument(before));
  }
  H.HTMLSelectElement_add = (s, element, before) => { selectAdd(s, element, before); };
  function removeOptionAt(s, index) {
    const list = htmlList(s, OPTIONS);
    if (index < 0 || index >= size(list)) return;
    const o = at(list, index);
    remove(apply(parentNode_, o, []), o);
  }
  // Two overloads of one name, which the generator doesn't do: WebIDL §3.7.7 by
  // argument count. With none it is ChildNode's.
  const childRemove = elementProto.remove;
  op(I_HTMLSelectElement, 'remove', function remove_() {
    const s = self(this, I_HTMLSelectElement);
    if (arguments.length === 0) return apply(childRemove, this, []);
    return removeOptionAt(s, arguments[0] | 0);
  });
  defineProperty(protos[I_HTMLSelectElement].remove, 'name', { value: 'remove', configurable: true });
  // Whether a script chose nothing, which no attribute can say. It holds until an
  // option is selected again.
  function clearedBy(s) {
    if (s.cleared !== true) return false;
    const list = htmlList(s, OPTIONS);
    const m = size(list);
    for (let i = 0; i < m; i++) {
      if (attrOf(stateOf(at(list, i)), 'selected') !== null) {
        s.cleared = false;
        return false;
      }
    }
    return true;
  }
  function selectedIndexOf(s) {
    if (clearedBy(s)) return -1;
    const chosen = htmlList(s, SELECTED);
    return size(chosen) === 0 ? -1 : indexIn(htmlList(s, OPTIONS), at(chosen, 0));
  }
  // Only one stays selected: `selected` is the attribute (src/html.rs).
  function selectOnly(s, keep) {
    const list = htmlList(s, OPTIONS);
    const m = size(list);
    for (let i = 0; i < m; i++) {
      const o = stateOf(at(list, i));
      if (o.w !== keep) call(OP_removeAttribute, o.g, o.h, 'selected');
    }
    s.cleared = keep === null;
    if (keep !== null) {
      const k = stateOf(keep);
      call(OP_setAttribute, k.g, k.h, 'selected', '');
    }
  }
  H.HTMLSelectElement_selectedIndex = selectedIndexOf;
  H.set_HTMLSelectElement_selectedIndex = (s, i) => { selectOnly(s, at(htmlList(s, OPTIONS), i)); };
  H.HTMLSelectElement_value = (s) => {
    if (clearedBy(s)) return '';
    const chosen = htmlList(s, SELECTED);
    const m = size(chosen);
    for (let i = 0; i < m; i++) {
      const o = stateOf(at(chosen, i));
      if (!optionDisabled(o)) return optionValue(o);
    }
    return '';
  };
  H.set_HTMLSelectElement_value = (s, v) => {
    const list = htmlList(s, OPTIONS);
    const m = size(list);
    let found = null;
    for (let i = 0; i < m && found === null; i++) {
      if (optionValue(stateOf(at(list, i))) === v) found = at(list, i);
    }
    selectOnly(s, found);
  };
  H.HTMLOptionsCollection_length = (l) => size(l.w);
  H.set_HTMLOptionsCollection_length = (l, v) => { setOptionCount(stateOf(listOwner(l)), v); };
  const listOwner = (l) => wrap(l.g, call(OP_listOwner, l.g, l.h));
  H.HTMLOptionsCollection_add = (l, element, before) => {
    selectAdd(stateOf(listOwner(l)), element, before);
  };
  H.HTMLOptionsCollection_remove = (l, index) => { removeOptionAt(stateOf(listOwner(l)), index); };
  H.HTMLOptionsCollection_selectedIndex = (l) => selectedIndexOf(stateOf(listOwner(l)));
  H.set_HTMLOptionsCollection_selectedIndex = (l, i) => {
    H.set_HTMLSelectElement_selectedIndex(stateOf(listOwner(l)), i);
  };

  function optionDisabled(o) {
    if (attrOf(o, 'disabled') !== null) return true;
    const p = apply(parentNode_, o.w, []);
    return p !== null && stateOf(p).m[I_Element] === true && isElement(p, 'optgroup') &&
      attrOf(stateOf(p), 'disabled') !== null;
  }
  // HTML §4.10.10: the descendant text but for scripts', white space stripped and
  // collapsed. Nodes are nested at most `max_depth`, so this recursion is short.
  function textWithoutScripts(node) {
    let out = '';
    for (let c = apply(firstChild_, node, []); c !== null; c = apply(nextSibling_, c, [])) {
      const type = apply(nodeType_, c, []);
      if (type === 3 || type === 4) out += apply(nodeValue_, c, []);
      else if (type === 1 && apply(localName_, c, []) !== 'script') out += textWithoutScripts(c);
    }
    return out;
  }
  function optionText(o) {
    const text = textWithoutScripts(o.w);
    let out = '';
    let space = false;
    for (let i = 0; i < text.length; i++) {
      const c = apply(charCodeAt, text, [i]);
      if (c === 32 || c === 9 || c === 10 || c === 12 || c === 13) {
        space = out !== '';
      } else {
        if (space) out += ' ';
        space = false;
        out += text[i];
      }
    }
    return out;
  }
  function optionValue(o) {
    const v = attrOf(o, 'value');
    return v === null ? optionText(o) : v;
  }
  H.HTMLOptionElement_form = (o) => {
    const s = selectOf(o.w);
    return s === null ? null : formOf(s);
  };
  H.HTMLOptionElement_label = (o) => {
    const v = attrOf(o, 'label');
    return v === null ? optionText(o) : v;
  };
  H.set_HTMLOptionElement_label = (o, v) => { setReflectStr(o, 'label', v); };
  H.HTMLOptionElement_value = (o) => optionValue(o);
  H.set_HTMLOptionElement_value = (o, v) => { setReflectStr(o, 'value', v); };
  H.HTMLOptionElement_text = optionText;
  H.set_HTMLOptionElement_text = (o, v) => { apply(textContent_.set, o.w, [v]); };
  H.HTMLOptionElement_index = (o) => {
    const s = selectOf(o.w);
    if (s === null) return 0;
    const i = indexIn(htmlList(s, OPTIONS), o.w);
    return i < 0 ? 0 : i;
  };
  H.HTMLOptionElement_selected = (o) => {
    const s = selectOf(o.w);
    if (s === null) return attrOf(o, 'selected') !== null;
    return indexIn(htmlList(s, SELECTED), o.w) >= 0;
  };
  H.set_HTMLOptionElement_selected = (o, v) => {
    const s = selectOf(o.w);
    if (v && s !== null && attrOf(s, 'multiple') === null) selectOnly(s, o.w);
    else setReflectBool(o, 'selected', v);
  };

  // HTML §4.10.7's legacy factories, and `Image`'s (§4.8.3). Their `prototype` is
  // the interface's, so `new Option() instanceof HTMLOptionElement`.
  function legacyFactory(name, id, f) {
    defineProperty(f, 'name', { value: name, configurable: true });
    defineProperty(f, 'prototype', { value: protos[id], writable: false });
    defineProperty(G, name, { value: f, writable: true, configurable: true });
  }
  const pageDocument = () => {
    const g = current();
    return g === 0 ? stale() : wrap(g, root());
  };
  legacyFactory('Option', I_HTMLOptionElement, function (text, value, defaultSelected, selected) {
    if (new.target === undefined) throw new TE('Constructor requires new');
    const w = apply(createElementNS_, pageDocument(), [XHTML, 'option']);
    const o = stateOf(w);
    if (text !== undefined) {
      const t = `${text}`;
      if (t !== '') apply(textContent_.set, w, [t]);
    }
    if (value !== undefined) setReflectStr(o, 'value', value);
    if (defaultSelected) call(OP_setAttribute, o.g, o.h, 'selected', '');
    if (selected) call(OP_setAttribute, o.g, o.h, 'selected', '');
    return w;
  });
  legacyFactory('Image', I_HTMLImageElement, function (width, height) {
    if (new.target === undefined) throw new TE('Constructor requires new');
    const w = apply(createElementNS_, pageDocument(), [XHTML, 'img']);
    const i = stateOf(w);
    if (width !== undefined) call(OP_setAttribute, i.g, i.h, 'width', `${width >>> 0}`);
    if (height !== undefined) call(OP_setAttribute, i.g, i.h, 'height', `${height >>> 0}`);
    return w;
  });

  // A list's handler, with an index setter for `options`. Everything else is
  // HTMLCollection's, whose handler holds the operations.
  exotics[I_HTMLFormControlsCollection] = exotics[I_HTMLCollection];
  {
    const base = exotics[I_HTMLCollection];
    const h = create(null);
    for (const k in base) h[k] = base[k];
    h.set = function (t, k, v, r) {
      const i = indexKey(k);
      if (i < 0 || stateOf(r) !== stateOf(t)) return base.set(t, k, v, r);
      setOption(stateOf(t), i, v);
      return true;
    };
    h.defineProperty = function (t, k, d) {
      const i = indexKey(k);
      if (i < 0) return base.defineProperty(t, k, d);
      setOption(stateOf(t), i, d.value);
      return true;
    };
    exotics[I_HTMLOptionsCollection] = h;
  }
  // The setter of HTMLOptionsCollection: past the end, blank options come first.
  function setOption(l, index, value) {
    const s = stateOf(listOwner(l));
    if (value === null) { removeOptionAt(s, index); return; }
    const o = stateOf(value);
    if (o === null || o.m[I_HTMLOptionElement] !== true) throw new TE('the value is not an option');
    const list = htmlList(s, OPTIONS);
    const m = size(list);
    if (index > m) setOptionCount(s, index);
    if (index >= m) {
      apply(appendChild_, s.w, [value]);
    } else {
      const old = at(list, index);
      apply(protos[I_Node].replaceChild, apply(parentNode_, old, []), [value, old]);
    }
  }

  // ------ Forms ------

  H.HTMLFormElement_elements = (s) => htmlList(s, CONTROLS);
  H.HTMLFormElement_length = (s) => size(htmlList(s, CONTROLS));
  // §4.10.3: the document's URL for no attribute or an empty one.
  H.HTMLFormElement_action = (s) => {
    const v = attrOf(s, 'action');
    const d = stateOf(apply(ownerDocument_, s.w, []));
    if (v === null || v === '') return call(OP_urlPart, d.g, d.h, 'href');
    const url = call(OP_urlJoin, d.g, d.h, v);
    return url === '' ? v : url;
  };
  H.set_HTMLFormElement_action = (s, v) => { setReflectStr(s, 'action', v); };
  // §2.6.1 enumerated attributes, with a default for a missing or unknown value.
  function enumerated(s, name, keywords, fallback) {
    const v = attrOf(s, name);
    if (v === null) return fallback;
    const low = asciiLower(v);
    for (let i = 0; i < keywords.length; i++) if (keywords[i] === low) return low;
    return fallback;
  }
  H.HTMLFormElement_method = (s) => enumerated(s, 'method', ['get', 'post', 'dialog'], 'get');
  H.set_HTMLFormElement_method = (s, v) => { setReflectStr(s, 'method', v); };
  const ENCODINGS = ['application/x-www-form-urlencoded', 'multipart/form-data', 'text/plain'];
  H.HTMLFormElement_enctype = (s) => enumerated(s, 'enctype', ENCODINGS, ENCODINGS[0]);
  H.set_HTMLFormElement_enctype = (s, v) => { setReflectStr(s, 'enctype', v); };
  H.HTMLFormElement_encoding = H.HTMLFormElement_enctype;
  H.set_HTMLFormElement_encoding = H.set_HTMLFormElement_enctype;
  // §4.10.11: what a script writes has to reach the printed page, as for `input`.
  H.HTMLTextAreaElement_value = (s) => apply(textContent_.get, s.w, []);
  H.set_HTMLTextAreaElement_value = (s, v) => { apply(textContent_.set, s.w, [v]); };
  H.HTMLTextAreaElement_defaultValue = H.HTMLTextAreaElement_value;
  H.set_HTMLTextAreaElement_defaultValue = H.set_HTMLTextAreaElement_value;

  // ------ Hyperlinks' URL decomposition (HTML §4.6.3) ------
  //
  // The `href` attribute parsed against the document, or nothing: a getter
  // answers empty, and a setter leaves the attribute.

  function linkUrl(s) {
    const v = attrOf(s, 'href');
    if (v === null) return null;
    const d = stateOf(apply(ownerDocument_, s.w, []));
    const url = call(OP_urlJoin, d.g, d.h, v);
    return url === '' ? null : url;
  }
  const linkGet = (part, empty) => (s) => {
    const url = linkUrl(s);
    return url === null ? empty : n.urlGet(url, part);
  };
  const linkSet = (part) => (s, v) => {
    const url = linkUrl(s);
    if (url === null) return;
    const next = n.urlSet(url, part, v);
    if (next !== undefined) call(OP_setAttribute, s.g, s.h, 'href', next);
  };
  H.HTMLAnchorElement_origin = linkGet('origin', '');
  H.HTMLAreaElement_origin = H.HTMLAnchorElement_origin;
  H.HTMLAnchorElement_protocol = linkGet('protocol', ':');
  H.HTMLAreaElement_protocol = H.HTMLAnchorElement_protocol;
  H.set_HTMLAnchorElement_protocol = linkSet('protocol');
  H.set_HTMLAreaElement_protocol = H.set_HTMLAnchorElement_protocol;
  H.HTMLAnchorElement_username = linkGet('username', '');
  H.HTMLAreaElement_username = H.HTMLAnchorElement_username;
  H.set_HTMLAnchorElement_username = linkSet('username');
  H.set_HTMLAreaElement_username = H.set_HTMLAnchorElement_username;
  H.HTMLAnchorElement_password = linkGet('password', '');
  H.HTMLAreaElement_password = H.HTMLAnchorElement_password;
  H.set_HTMLAnchorElement_password = linkSet('password');
  H.set_HTMLAreaElement_password = H.set_HTMLAnchorElement_password;
  H.HTMLAnchorElement_host = linkGet('host', '');
  H.HTMLAreaElement_host = H.HTMLAnchorElement_host;
  H.set_HTMLAnchorElement_host = linkSet('host');
  H.set_HTMLAreaElement_host = H.set_HTMLAnchorElement_host;
  H.HTMLAnchorElement_hostname = linkGet('hostname', '');
  H.HTMLAreaElement_hostname = H.HTMLAnchorElement_hostname;
  H.set_HTMLAnchorElement_hostname = linkSet('hostname');
  H.set_HTMLAreaElement_hostname = H.set_HTMLAnchorElement_hostname;
  H.HTMLAnchorElement_port = linkGet('port', '');
  H.HTMLAreaElement_port = H.HTMLAnchorElement_port;
  H.set_HTMLAnchorElement_port = linkSet('port');
  H.set_HTMLAreaElement_port = H.set_HTMLAnchorElement_port;
  H.HTMLAnchorElement_pathname = linkGet('pathname', '');
  H.HTMLAreaElement_pathname = H.HTMLAnchorElement_pathname;
  H.set_HTMLAnchorElement_pathname = linkSet('pathname');
  H.set_HTMLAreaElement_pathname = H.set_HTMLAnchorElement_pathname;
  H.HTMLAnchorElement_search = linkGet('search', '');
  H.HTMLAreaElement_search = H.HTMLAnchorElement_search;
  H.set_HTMLAnchorElement_search = linkSet('search');
  H.set_HTMLAreaElement_search = H.set_HTMLAnchorElement_search;
  H.HTMLAnchorElement_hash = linkGet('hash', '');
  H.HTMLAreaElement_hash = H.HTMLAnchorElement_hash;
  H.set_HTMLAnchorElement_hash = linkSet('hash');
  H.set_HTMLAreaElement_hash = H.set_HTMLAnchorElement_hash;
  H.HTMLAnchorElement_text = (s) => apply(textContent_.get, s.w, []);
  H.set_HTMLAnchorElement_text = (s, v) => { apply(textContent_.set, s.w, [v]); };

  // ------ The document's cookie (HTML §3.1.3) ------
  //
  // No cookie store exists, as for a document in a sandbox with nothing to read.

  H.Document_cookie = () => '';
  H.set_Document_cookie = () => {};

  // ------ Focus (HTML §6.6) ------
  //
  // One state per document, in its record. Nothing lays out, so "being rendered"
  // is being connected; what can take focus is what §6.6.2 lists as a focusable
  // area without layout. `:focus` doesn't change.

  function focusable(s) {
    if (!connected(s)) return false;
    if (attrOf(s, 'tabindex') !== null) return true;
    switch (localOf(s)) {
      case 'a': case 'area': return attrOf(s, 'href') !== null;
      case 'input':
        if (attrOf(s, 'type') !== null && asciiLower(attrOf(s, 'type')) === 'hidden') return false;
        return !isDisabled(s);
      case 'button': case 'select': case 'textarea': return !isDisabled(s);
      case 'iframe': case 'summary': return true;
      default: return false;
    }
  }
  function fireFocus(target, type, related) {
    const e = make(I_FocusEvent);
    initEvent(e, type, type === 'focusin' || type === 'focusout', false);
    e.isTrusted = true;
    e.composed = true;
    e.d = eventInits[I_FocusEvent]();
    e.d.relatedTarget = related;
    dispatch(target, e);
  }
  const documentOf = (s) => stateOf(apply(ownerDocument_, s.w, []));
  // What holds focus now: an element that has left the tree doesn't.
  function focused(d) {
    if (d.active !== undefined && d.active !== null && !connected(d.active)) d.active = null;
    return d.active === undefined ? null : d.active;
  }
  function changeFocus(d, next) {
    const old = focused(d);
    if (old === next) return;
    d.active = next;
    const from = old === null ? null : old.w, to = next === null ? null : next.w;
    if (old !== null) {
      fireFocus(old, 'blur', to);
      fireFocus(old, 'focusout', to);
    }
    if (next !== null && d.active === next) {
      fireFocus(next, 'focus', from);
      fireFocus(next, 'focusin', from);
    }
  }
  function focusElement(s) {
    if (focusable(s)) changeFocus(documentOf(s), s);
  }
  function blurElement(s) {
    const d = documentOf(s);
    if (focused(d) === s) changeFocus(d, null);
  }
  H.HTMLElement_focus = focusElement;
  H.HTMLElement_blur = blurElement;
  H.SVGElement_focus = focusElement;
  H.SVGElement_blur = blurElement;
  // §6.6.4: a parsed `tabindex`, or 0 for what is focusable by default.
  function tabIndexOf(s) {
    const v = attrOf(s, 'tabindex');
    if (v !== null) {
      const x = parseInt_(v, 10);
      if (x === x) return x | 0;
    }
    switch (localOf(s)) {
      case 'a': case 'area': case 'button': case 'iframe': case 'input': case 'object':
      case 'select': case 'textarea': case 'summary':
        return 0;
      default:
        return -1;
    }
  }
  const setTabIndex = (s, v) => { call(OP_setAttribute, s.g, s.h, 'tabindex', `${v}`); };
  H.HTMLElement_tabIndex = tabIndexOf;
  H.set_HTMLElement_tabIndex = setTabIndex;
  H.SVGElement_tabIndex = tabIndexOf;
  H.set_SVGElement_tabIndex = setTabIndex;
  H.Document_activeElement = (d) => {
    const f = focused(d);
    if (f !== null) return f.w;
    return wrap(d.g, call(OP_body, d.g, d.h));
  };
  H.Document_hasFocus = (d) => isPage(d);
