// What a page script gets from layout queries in phase 2, where nothing is laid
// out: zeros (a page that only reads sizes carries on) or a thrown error.
// `mode` is set by the code around this file. A member crates/dom binds wins, so
// when real layout arrives in phase 3 each one retires on its own.
//
// This is a script of the page: it runs first, in the page's realm, and takes
// itself out of the tree, so no page sees it there. A page can replace anything
// here, as it can in a browser.
const refuse = (name) => () => {
  throw new DOMException(
    `${name} needs layout, which isn't available (layout queries: throw)`,
    'NotSupportedError',
  );
};
const define = (target, name, answer) => {
  if (!target || name in target) return;
  Object.defineProperty(target, name, {
    configurable: true,
    enumerable: true,
    get: mode === 'throw' ? refuse(name) : () => answer,
    set() {},
  });
};
const method = (target, name, answer) => {
  if (!target || name in target) return;
  Object.defineProperty(target, name, {
    configurable: true,
    enumerable: true,
    writable: true,
    value: mode === 'throw' ? refuse(`${name}()`) : answer,
  });
};
const element = window.Element && Element.prototype;
const html = window.HTMLElement && HTMLElement.prototype;
method(element, 'getBoundingClientRect', function getBoundingClientRect() {
  return Object.freeze({
    x: 0, y: 0, width: 0, height: 0, top: 0, right: 0, bottom: 0, left: 0,
    toJSON() {
      return { x: 0, y: 0, width: 0, height: 0, top: 0, right: 0, bottom: 0, left: 0 };
    },
  });
});
method(element, 'getClientRects', function getClientRects() {
  return [];
});
for (const name of ['clientWidth', 'clientHeight', 'clientTop', 'clientLeft',
  'scrollWidth', 'scrollHeight', 'scrollTop', 'scrollLeft']) {
  define(element, name, 0);
}
for (const name of ['offsetWidth', 'offsetHeight', 'offsetTop', 'offsetLeft']) {
  define(html, name, 0);
}
define(html, 'offsetParent', null);
define(window, 'innerWidth', 0);
define(window, 'innerHeight', 0);
method(window, 'getComputedStyle', function getComputedStyle() {
  return Object.freeze({ getPropertyValue() { return ''; } });
});
const self = document.currentScript;
self.parentNode.removeChild(self);
