
  // ------ Layout queries (brief, phase 2): the policy, until phase 3 ------
  //
  // There is no layout yet. Each query asks Rust first, which ends the call
  // with DomError::LayoutQuery under LayoutPolicy::Throw; under the default,
  // LayoutPolicy::Zeros, it answers as a browser does for an element with no
  // box: zeros, empty lists, a null offsetParent, an empty computed style.

  const mathMin = Math.min, mathMax = Math.max;

  function askElement(s, name) { call(OP_layoutQuery, s.g, s.h, name); }
  function askDocument(name) {
    const g = current();
    if (g !== 0) call(OP_layoutQuery, g, root(), name);
  }

  // DOMRectReadOnly and DOMRect (Geometry §6).
  function newRect(id, target, x, y, width, height) {
    const s = make(id, target);
    // In argument order, before the object can be seen.
    s.x = +x; s.y = +y; s.width = +width; s.height = +height;
    return s.w;
  }
  function rectFrom(id, d) {
    const o = dictionary(d, 'fromRect');
    // WebIDL §3.2.17: dictionary members in lexicographic order.
    const field = (k) => { const v = o[k]; return v === undefined ? 0 : +v; };
    const height = field('height'), width = field('width'), x = field('x'), y = field('y');
    return newRect(id, undefined, x, y, width, height);
  }
  function fromRectOn(id) {
    defineProperty(ctors[id], 'fromRect', {
      value: { fromRect(other) { return rectFrom(id, other); } }.fromRect,
      writable: true, enumerable: true, configurable: true,
    });
    defineProperty(ctors[id].fromRect, 'length', { value: 0, configurable: true });
  }
  fromRectOn(I_DOMRectReadOnly);
  fromRectOn(I_DOMRect);
  H.DOMRectReadOnly_toJSON = (s) => {
    const o = {};
    const put = (k, v) => {
      const d = create(null);
      d.value = v; d.writable = true; d.enumerable = true; d.configurable = true;
      defineProperty(o, k, d);
    };
    put('x', s.x); put('y', s.y); put('width', s.width); put('height', s.height);
    put('top', H.DOMRectReadOnly_top(s)); put('right', H.DOMRectReadOnly_right(s));
    put('bottom', H.DOMRectReadOnly_bottom(s)); put('left', H.DOMRectReadOnly_left(s));
    return o;
  };
  H.new_DOMRectReadOnly = (target, x, y, w, h) => newRect(I_DOMRectReadOnly, target, x, y, w, h);
  H.new_DOMRect = (target, x, y, w, h) => newRect(I_DOMRect, target, x, y, w, h);
  for (const k of ['x', 'y', 'width', 'height']) {
    H['DOMRectReadOnly_' + k] = (s) => s[k];
    H['DOMRect_' + k] = (s) => s[k];
    H['set_DOMRect_' + k] = (s, v) => { s[k] = +v; };
  }
  H.DOMRectReadOnly_top = (s) => mathMin(s.y, s.y + s.height);
  H.DOMRectReadOnly_right = (s) => mathMax(s.x, s.x + s.width);
  H.DOMRectReadOnly_bottom = (s) => mathMax(s.y, s.y + s.height);
  H.DOMRectReadOnly_left = (s) => mathMin(s.x, s.x + s.width);

  // Always empty until there are boxes, so it needs no indexed properties.
  H.DOMRectList_length = () => 0;
  op(I_DOMRectList, 'item', function item(index) {
    self(this, I_DOMRectList);
    need(arguments.length, 1, 'item');
    return null;
  });
  defineProperty(protos[I_DOMRectList], symbolIterator, {
    value: arrayValues, writable: true, configurable: true,
  });

  op(I_Element, 'getBoundingClientRect', function getBoundingClientRect() {
    askElement(self(this, I_Element), 'getBoundingClientRect');
    return newRect(I_DOMRect, undefined, 0, 0, 0, 0);
  });
  op(I_Element, 'getClientRects', function getClientRects() {
    askElement(self(this, I_Element), 'getClientRects');
    return make(I_DOMRectList).w;
  });
  for (const name of ['clientTop', 'clientLeft', 'clientWidth', 'clientHeight', 'scrollWidth', 'scrollHeight']) {
    attr(I_Element, name, function () { askElement(self(this, I_Element), name); return 0; });
  }
  for (const name of ['scrollTop', 'scrollLeft']) {
    attr(I_Element, name, function () { askElement(self(this, I_Element), name); return 0; },
      function (v) { askElement(self(this, I_Element), name); +v; });
  }
  for (const name of ['offsetTop', 'offsetLeft', 'offsetWidth', 'offsetHeight']) {
    attr(I_HTMLElement, name, function () { askElement(self(this, I_HTMLElement), name); return 0; });
  }
  attr(I_HTMLElement, 'offsetParent', function () {
    askElement(self(this, I_HTMLElement), 'offsetParent');
    return null;
  });
  op(I_Document, 'elementFromPoint', function elementFromPoint(x, y) {
    const s = self(this, I_Document);
    need(arguments.length, 2, 'elementFromPoint');
    +x; +y;
    askElement(s, 'elementFromPoint');
    return null;
  });
  // [Replaceable]: a write replaces the attribute with an own property.
  for (const name of ['innerWidth', 'innerHeight', 'outerWidth', 'outerHeight',
    'scrollX', 'scrollY', 'pageXOffset', 'pageYOffset']) {
    attr(I_Window, name, function () { self(this, I_Window); askDocument(name); return 0; },
      function (v) {
        self(this, I_Window);
        const d = create(null);
        d.value = v; d.writable = true; d.enumerable = true; d.configurable = true;
        defineProperty(G, name, d);
      });
  }
  // CSSOM §9.1. The pseudo-element argument changes nothing while the style
  // is empty, but it is still converted.
  op(I_Window, 'getComputedStyle', function getComputedStyle(elt) {
    self(this, I_Window);
    need(arguments.length, 1, 'getComputedStyle');
    const s = stateOf(elt);
    if (s === null || !is(s, I_Element)) throw new TE('Argument is not a Element');
    if (arguments.length > 1 && arguments[1] !== undefined && arguments[1] !== null) str(arguments[1]);
    return wrap(s.g, call(OP_computedStyle, s.g, s.h));
  });
