// Runs in a fresh context after a page has settled, so nothing a page did to
// prototypes or globals reaches it: it sees the tree, and the bindings as
// generated. `keepScripts` is set by the code around this file.
const removeAll = (list, skip) => {
  // Last first: removing a child is cheapest at the end of its parent's list.
  for (let i = list.length - 1; i >= 0; i--) {
    const node = list[i];
    if (!skip(node) && node.parentNode) node.parentNode.removeChild(node);
  }
};
const isIsland = (script) => {
  const type = script.getAttribute('type');
  return type !== null
    && type.split(';')[0].trim().toLowerCase() === 'application/vnd.surimono+json';
};
// The printer parses with scripting off, so it would print what's in here.
removeAll(document.querySelectorAll('noscript'), () => false);
if (!keepScripts) removeAll(document.querySelectorAll('script'), isIsland);
const images = document.querySelectorAll('img[src]:not([src^="data:"])');
const urls = [];
for (let i = 0; i < images.length && i < 64; i++) urls.push(images[i].getAttribute('src'));
return JSON.stringify({
  standards: document.compatMode === 'CSS1Compat',
  images: images.length,
  urls,
});
