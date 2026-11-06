  // ------ NodeIterator and TreeWalker (DOM §6) ------
  //
  // A filter is a script's function, so the walks are here, over the DOM's own
  // getters, captured now. A filter can do anything to the tree between two steps,
  // and each step reads the tree again. A NodeIterator's pointer is in Rust, which
  // sees removals (src/traversal.rs); a TreeWalker's is here, as nothing moves it.

  const FILTER_ACCEPT = 1, FILTER_REJECT = 2, FILTER_SKIP = 3;
  const firstChild_ = rdescriptor(nodeProto, 'firstChild').get;
  const lastChild_ = rdescriptor(nodeProto, 'lastChild').get;
  const nextSibling_ = rdescriptor(nodeProto, 'nextSibling').get;
  const previousSibling_ = rdescriptor(nodeProto, 'previousSibling').get;
  const nodeType_ = rdescriptor(nodeProto, 'nodeType').get;
  const get = (getter, node) => apply(getter, node, []);

  // DOM §6 "filter": `t` is the iterator's or walker's record.
  function nodeFilter(t, node) {
    if (t.active) throwDom('InvalidStateError', 'the filter is already running');
    if (((1 << (get(nodeType_, node) - 1)) & t.show) === 0) return FILTER_SKIP;
    const f = t.filter;
    if (f === null) return FILTER_ACCEPT;
    t.active = true;
    let result;
    try {
      // WebIDL §3.11 "call a user object's operation".
      if (typeof f === 'function') {
        result = apply(f, undefined, [node]);
      } else {
        const accept = f.acceptNode;
        if (typeof accept !== 'function') throw new TE('the filter has no acceptNode');
        result = apply(accept, f, [node]);
      }
    } finally {
      t.active = false;
    }
    return (result >>> 0) & 65535;
  }
  function traversal(id, root, whatToShow, filter) {
    const t = make(id);
    t.root = root.w; t.show = whatToShow; t.filter = filter === undefined ? null : filter;
    t.active = false;
    return t;
  }

  H.Document_createNodeIterator = (d, root, whatToShow, filter) => {
    const t = traversal(I_NodeIterator, root, whatToShow, filter);
    t.at = call(OP_iteratorNew, root.g, root.h);
    t.s = root;
    return t.w;
  };
  H.Document_createTreeWalker = (d, root, whatToShow, filter) => {
    const t = traversal(I_TreeWalker, root, whatToShow, filter);
    t.current = root.w;
    return t.w;
  };
  H.NodeIterator_root = (t) => t.root;
  H.NodeIterator_whatToShow = (t) => t.show;
  H.NodeIterator_filter = (t) => t.filter;
  const iteratorNode = (t) => wrap(t.s.g, call(OP_iteratorReference, t.s.g, t.s.h, t.at));
  H.NodeIterator_referenceNode = iteratorNode;
  H.NodeIterator_pointerBeforeReferenceNode = (t) => call(OP_iteratorBefore, t.s.g, t.s.h, t.at);
  H.NodeIterator_detach = () => {};
  // The node after `node` in tree order, within `root`.
  function following(node, root) {
    const c = get(firstChild_, node);
    if (c !== null) return c;
    for (let at = node; at !== null; at = get(parentNode_, at)) {
      if (at === root) return null;
      const s = get(nextSibling_, at);
      if (s !== null) return s;
    }
    return null;
  }
  function preceding(node, root) {
    if (node === root) return null;
    let at = get(previousSibling_, node);
    if (at === null) return get(parentNode_, node);
    for (let c = get(lastChild_, at); c !== null; c = get(lastChild_, at)) at = c;
    return at;
  }
  // §6.1 "traverse".
  function iterate(t, next) {
    let node = iteratorNode(t);
    let before = call(OP_iteratorBefore, t.s.g, t.s.h, t.at);
    for (;;) {
      if (next) {
        if (before) {
          before = false;
        } else {
          node = following(node, t.root);
          if (node === null) return null;
        }
      } else if (before) {
        node = preceding(node, t.root);
        if (node === null) return null;
      } else {
        before = true;
      }
      if (nodeFilter(t, node) === FILTER_ACCEPT) break;
    }
    const n_ = stateOf(node);
    call(OP_iteratorSet, t.s.g, t.s.h, t.at, n_.h, before);
    return node;
  }
  H.NodeIterator_nextNode = (t) => iterate(t, true);
  H.NodeIterator_previousNode = (t) => iterate(t, false);

  H.TreeWalker_root = (t) => t.root;
  H.TreeWalker_whatToShow = (t) => t.show;
  H.TreeWalker_filter = (t) => t.filter;
  H.TreeWalker_currentNode = (t) => t.current;
  H.set_TreeWalker_currentNode = (t, v) => { t.current = v.w; };
  H.TreeWalker_parentNode = (t) => {
    let node = t.current;
    while (node !== null && node !== t.root) {
      node = get(parentNode_, node);
      if (node !== null && nodeFilter(t, node) === FILTER_ACCEPT) {
        t.current = node;
        return node;
      }
    }
    return null;
  };
  // §6.2 "traverse children".
  function walkChildren(t, first) {
    const inward = first ? firstChild_ : lastChild_;
    const across = first ? nextSibling_ : previousSibling_;
    let node = get(inward, t.current);
    while (node !== null) {
      const result = nodeFilter(t, node);
      if (result === FILTER_ACCEPT) {
        t.current = node;
        return node;
      }
      if (result === FILTER_SKIP) {
        const child = get(inward, node);
        if (child !== null) {
          node = child;
          continue;
        }
      }
      while (node !== null) {
        const sibling = get(across, node);
        if (sibling !== null) {
          node = sibling;
          break;
        }
        const parent = get(parentNode_, node);
        if (parent === null || parent === t.root || parent === t.current) return null;
        node = parent;
      }
    }
    return null;
  }
  H.TreeWalker_firstChild = (t) => walkChildren(t, true);
  H.TreeWalker_lastChild = (t) => walkChildren(t, false);
  // §6.2 "traverse siblings".
  function walkSiblings(t, forward) {
    let node = t.current;
    if (node === t.root) return null;
    const across = forward ? nextSibling_ : previousSibling_;
    const inward = forward ? firstChild_ : lastChild_;
    for (;;) {
      let sibling = get(across, node);
      while (sibling !== null) {
        node = sibling;
        const result = nodeFilter(t, node);
        if (result === FILTER_ACCEPT) {
          t.current = node;
          return node;
        }
        sibling = get(inward, node);
        if (result === FILTER_REJECT || sibling === null) sibling = get(across, node);
      }
      node = get(parentNode_, node);
      if (node === null || node === t.root) return null;
      if (nodeFilter(t, node) === FILTER_ACCEPT) return null;
    }
  }
  H.TreeWalker_previousSibling = (t) => walkSiblings(t, false);
  H.TreeWalker_nextSibling = (t) => walkSiblings(t, true);
  H.TreeWalker_previousNode = (t) => {
    let node = t.current;
    while (node !== t.root) {
      let sibling = get(previousSibling_, node);
      while (sibling !== null) {
        node = sibling;
        let result = nodeFilter(t, node);
        while (result !== FILTER_REJECT && get(lastChild_, node) !== null) {
          node = get(lastChild_, node);
          result = nodeFilter(t, node);
        }
        if (result === FILTER_ACCEPT) {
          t.current = node;
          return node;
        }
        sibling = get(previousSibling_, node);
      }
      if (node === t.root || get(parentNode_, node) === null) return null;
      node = get(parentNode_, node);
      if (nodeFilter(t, node) === FILTER_ACCEPT) {
        t.current = node;
        return node;
      }
    }
    return null;
  };
  H.TreeWalker_nextNode = (t) => {
    let node = t.current;
    let result = FILTER_ACCEPT;
    for (;;) {
      while (result !== FILTER_REJECT && get(firstChild_, node) !== null) {
        node = get(firstChild_, node);
        result = nodeFilter(t, node);
        if (result === FILTER_ACCEPT) {
          t.current = node;
          return node;
        }
      }
      let sibling = null;
      for (let at = node; at !== null; at = get(parentNode_, at)) {
        if (at === t.root) return null;
        sibling = get(nextSibling_, at);
        if (sibling !== null) {
          node = sibling;
          break;
        }
      }
      if (sibling === null) return null;
      result = nodeFilter(t, node);
      if (result === FILTER_ACCEPT) {
        t.current = node;
        return node;
      }
    }
  };

  // A legacy callback interface object (WebIDL §3.13): constants on a function no
  // one can call.
  {
    const NodeFilter = function NodeFilter() { throw new TE('Illegal constructor'); };
    delete NodeFilter.prototype;
    for (const [name, value] of [
      ['FILTER_ACCEPT', 1], ['FILTER_REJECT', 2], ['FILTER_SKIP', 3], ['SHOW_ALL', 0xffffffff],
      ['SHOW_ELEMENT', 1], ['SHOW_ATTRIBUTE', 2], ['SHOW_TEXT', 4], ['SHOW_CDATA_SECTION', 8],
      ['SHOW_ENTITY_REFERENCE', 16], ['SHOW_ENTITY', 32], ['SHOW_PROCESSING_INSTRUCTION', 64],
      ['SHOW_COMMENT', 128], ['SHOW_DOCUMENT', 256], ['SHOW_DOCUMENT_TYPE', 512],
      ['SHOW_DOCUMENT_FRAGMENT', 1024], ['SHOW_NOTATION', 2048],
    ]) {
      defineProperty(NodeFilter, name, { value, enumerable: true });
    }
    defineProperty(G, 'NodeFilter', { value: NodeFilter, writable: true, configurable: true });
  }
