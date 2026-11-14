
  // ------ Layout queries (brief, phase 2): the policy, until phase 3 ------
  //
  // There is no layout yet. Each query asks Rust first, which ends the call
  // with DomError::LayoutQuery under LayoutPolicy::Throw; under the default,
  // LayoutPolicy::Zeros, it answers as a browser does for an element with no
  // box: zeros, empty lists, a null offsetParent, an empty computed style.

  const mathMin = Math.min, mathMax = Math.max;

  // WebIDL §3.2.7: a restricted `double` refuses NaN and the infinities.
  function restricted(v) {
    const n = +v;
    if (n - n !== 0) throw new TE('The value is not a finite number');
    return n;
  }

  // True when Rust has real numbers (LayoutPolicy::Real, the phase-3 prototype).
  function askElement(s, name) { return call(OP_layoutQuery, s.g, s.h, name); }
  // One number of the element's box: 0 to 3 the border box's x, y, width and
  // height in viewport coordinates, 4 and 5 offsetWidth and offsetHeight, 6
  // and 7 offsetLeft and offsetTop.
  // Natives return primitives only, so a rectangle is four calls; the layout
  // behind them runs once.
  function boxNumber(s, which) { return call(OP_layoutNumber, s.g, s.h, which); }
  function askDocument(name) {
    const g = current();
    if (g !== 0) call(OP_layoutQuery, g, root(), name);
  }

  // DOMRectReadOnly and DOMRect (Geometry §3), DOMRectList (§4).
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
  H.DOMRectReadOnly_x = (s) => s.x;
  H.DOMRectReadOnly_y = (s) => s.y;
  H.DOMRectReadOnly_width = (s) => s.width;
  H.DOMRectReadOnly_height = (s) => s.height;
  H.DOMRect_x = (s) => s.x;
  H.DOMRect_y = (s) => s.y;
  H.DOMRect_width = (s) => s.width;
  H.DOMRect_height = (s) => s.height;
  H.set_DOMRect_x = (s, v) => { s.x = +v; };
  H.set_DOMRect_y = (s, v) => { s.y = +v; };
  H.set_DOMRect_width = (s, v) => { s.width = +v; };
  H.set_DOMRect_height = (s, v) => { s.height = +v; };
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
    const s = self(this, I_Element);
    if (!askElement(s, 'getBoundingClientRect')) return newRect(I_DOMRect, undefined, 0, 0, 0, 0);
    return newRect(I_DOMRect, undefined,
      boxNumber(s, 0), boxNumber(s, 1), boxNumber(s, 2), boxNumber(s, 3));
  });
  op(I_Element, 'getClientRects', function getClientRects() {
    askElement(self(this, I_Element), 'getClientRects');
    return make(I_DOMRectList).w;
  });
  // CSSOM View §6.1 and SVG 2 §5.2. Nothing has a box, so nothing moves and
  // nothing is hit; `Throw` ends the call, as for every query.
  // `(ScrollToOptions)` or `(x, y)`: a lone argument is a dictionary, and two
  // are numbers. Named as WebIDL names an operation.
  function scrollArguments(a, name) {
    if (a.length === 1) dictionary(a[0], name);
    else if (a.length > 1) { +a[0]; +a[1]; }
  }
  for (const name of ['scroll', 'scrollTo', 'scrollBy']) {
    op(I_Element, name, { [name]() {
      const s = self(this, I_Element);
      scrollArguments(arguments, name);
      askElement(s, name);
    } }[name]);
    op(I_Window, name, { [name]() {
      self(this, I_Window);
      scrollArguments(arguments, name);
      askDocument(name);
    } }[name]);
  }
  op(I_Element, 'scrollIntoView', function scrollIntoView() {
    askElement(self(this, I_Element), 'scrollIntoView');
  });
  op(I_Document, 'elementsFromPoint', function elementsFromPoint(x, y) {
    const s = self(this, I_Document);
    need(arguments.length, 2, 'elementsFromPoint');
    restricted(x); restricted(y);
    askElement(s, 'elementsFromPoint');
    return [];
  });
  op(I_SVGElement, 'getBBox', function getBBox() {
    askElement(self(this, I_SVGElement), 'getBBox');
    return newRect(I_DOMRect, undefined, 0, 0, 0, 0);
  });
  // CSSOM View §6.3 on ranges: the same answers, and phase 3 the real ones.
  op(I_Range, 'getBoundingClientRect', function getBoundingClientRect() {
    askElement(self(this, I_Range), 'getBoundingClientRect');
    return newRect(I_DOMRect, undefined, 0, 0, 0, 0);
  });
  op(I_Range, 'getClientRects', function getClientRects() {
    askElement(self(this, I_Range), 'getClientRects');
    return make(I_DOMRectList).w;
  });
  for (const name of ['clientTop', 'clientLeft', 'clientWidth', 'clientHeight', 'scrollWidth', 'scrollHeight']) {
    attr(I_Element, name, function () { askElement(self(this, I_Element), name); return 0; });
  }
  for (const name of ['scrollTop', 'scrollLeft']) {
    attr(I_Element, name, function () { askElement(self(this, I_Element), name); return 0; },
      function (v) { askElement(self(this, I_Element), name); +v; });
  }
  // 6 and 7: offsetLeft and offsetTop, from the engines that answer them.
  for (const [name, which] of [['offsetTop', 7], ['offsetLeft', 6]]) {
    attr(I_HTMLElement, name, function () {
      const s = self(this, I_HTMLElement);
      return askElement(s, name) ? boxNumber(s, which) : 0;
    });
  }
  attr(I_HTMLElement, 'offsetWidth', function () {
    const s = self(this, I_HTMLElement);
    return askElement(s, 'offsetWidth') ? boxNumber(s, 4) : 0;
  });
  attr(I_HTMLElement, 'offsetHeight', function () {
    const s = self(this, I_HTMLElement);
    return askElement(s, 'offsetHeight') ? boxNumber(s, 5) : 0;
  });
  attr(I_HTMLElement, 'offsetParent', function () {
    askElement(self(this, I_HTMLElement), 'offsetParent');
    return null;
  });
  op(I_Document, 'elementFromPoint', function elementFromPoint(x, y) {
    const s = self(this, I_Document);
    need(arguments.length, 2, 'elementFromPoint');
    restricted(x); restricted(y);
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
  // CSSOM §7.2. [NewObject]: a new wrapper each call, over the element's one
  // handle, since handles are never reused. The pseudo-element argument
  // changes nothing while the style is empty, but it is still converted.
  op(I_Window, 'getComputedStyle', function getComputedStyle(elt) {
    self(this, I_Window);
    need(arguments.length, 1, 'getComputedStyle');
    const s = stateOf(elt);
    if (s === null || !is(s, I_Element)) throw new TE('Argument is not a Element');
    if (arguments.length > 1 && arguments[1] !== undefined && arguments[1] !== null) str(arguments[1]);
    return wrapper(s.g, call(OP_computedStyle, s.g, s.h));
  });
