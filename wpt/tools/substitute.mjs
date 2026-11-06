// wptserve's `sub` pipe for files with `.sub.` in their name, as
// wpt/README.md, "Substitution", describes. wpt/runner/src/main.rs has the same
// rules in Rust; sub-vectors.json holds both to what wptserve's pipes.py says.

export const HOST = 'web-platform.test';
export const ALT_HOST = 'not-web-platform.test';
/** tools/serve/serve.py's ports. Its "auto" entries are fixed here, so output doesn't vary. */
export const PORTS = {
  http: [8000, 8001], 'http-local': [8082], 'http-public': [8083],
  https: [8443, 8444], 'https-local': [8445], 'https-public': [8446],
  h2: [9000], ws: [8888], wss: [8889], 'webtransport-h3': [11000], dns: [8053],
};
const SUBDOMAINS = new Set(['', 'www', 'www1', 'www2', 'nonexistent']);
/** What `uuid()` gives. wptserve's is random; a test tool's output stays the same. */
export const UUID = '00000000-0000-4000-8000-000000000000';

const domain = (host, sub) => {
  if (!SUBDOMAINS.has(sub)) throw new Error(`no subdomain ${sub}`);
  return sub ? `${sub}.${host}` : host;
};

function tokens(text) {
  const out = [];
  const re = /\$\w+:|\$?\w+|\[[^\]]*\]|\([^)]*\)/y;
  for (let m = re.exec(text); m; m = re.exec(text)) out.push(m[0]);
  return out;
}

function location(url, key) {
  const parts = {
    server: url.origin, scheme: url.protocol.slice(0, -1), host: url.host, hostname: url.hostname,
    port: url.port, path: url.pathname, pathname: url.pathname, query: url.search || '?',
  };
  if (!(key in parts)) throw new Error(`no location[${key}]`);
  return parts[key];
}

function evaluate(body, url, variables) {
  const list = tokens(body);
  let name = list.shift();
  let variable;
  if (name?.endsWith(':')) {
    variable = name.slice(0, -1);
    name = list.shift();
  }
  if (name === undefined || !/^\$?\w+$/.test(name)) throw new Error(`bad substitution ${body}`);
  const index = () => {
    const t = list.shift();
    if (!t?.startsWith('[')) throw new Error(`${name} needs an index`);
    return t.slice(1, -1);
  };
  let value;
  if (Object.hasOwn(variables, name)) value = variables[name];
  else if (name === 'host') value = HOST;
  else if (name === 'url_base') value = '/';
  else if (name === 'domains') value = domain(HOST, index());
  else if (name === 'hosts') {
    const group = index();
    if (group !== '' && group !== 'alt') throw new Error(`no hosts[${group}]`);
    value = domain(group ? ALT_HOST : HOST, index());
  } else if (name === 'ports') {
    const protocol = index();
    const number = index();
    // Python's int(): spaces and a sign pass, and a negative index counts from the end.
    if (Object.hasOwn(PORTS, protocol) && /^\s*[+-]?\d+\s*$/.test(number)) value = PORTS[protocol].at(Number(number));
  } else if (name === 'location') value = location(url, index());
  else if (name === 'GET') value = url.searchParams.get(index()) ?? '';
  else if (name === 'uuid') value = list.shift() === '()' ? UUID : undefined;
  else if (name === 'header_or_default') {
    const args = list.shift();
    const given = args?.startsWith('(') ? args.slice(1, -1).split(/,\s*/) : [];
    if (given.length === 2) value = given[1];
  }
  if (value === undefined || list.length) throw new Error(`can't substitute ${body}`);
  value = String(value);
  if (variable) variables[variable] = value;
  return value;
}

const ESCAPES = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#x27;' };

const MARKUP = /\.(?:html|htm|xht|xhtml|xml|svg)$/;

/**
 * `text` with its `{{…}}` replaced for a request of `url`, a URL. Throws where wptserve does,
 * and on what isn't supported. Values are escaped in markup, by the extension of `file`
 * (handlers.py, `wrap_pipeline`), which is the URL's path unless a wrapper page names its script.
 */
export function substitute(text, url, file = url.pathname) {
  const variables = {};
  const escaped = MARKUP.test(file);
  return text.replace(/\{\{([^}]*)\}\}/g, (all, body) => {
    const value = evaluate(body, url, variables);
    return escaped ? value.replace(/[&<>"']/g, (c) => ESCAPES[c]) : value;
  });
}
