// structuredClone for bundles that expect the browser/Node global (Vega-Lite
// clones specs and configs with it). Defined only where it is missing.
//
// Follows the HTML structured clone algorithm for the values a renderer
// can meet: primitives, plain objects and arrays (own enumerable string
// keys, cycles and shared references kept), Boolean/Number/String/BigInt
// wrappers, Date, RegExp, Map, Set, ArrayBuffer and its views, and Error.
// Class instances come back as plain objects, as in the browser. Functions,
// symbols, promises, weak collections and other host objects throw a
// DataCloneError.

const toTag = Object.prototype.toString;
const getPrototypeOf = Object.getPrototypeOf;
const ownKeys = Object.keys;

function dataCloneError(what) {
  return new DOMException(`${what} could not be cloned.`, 'DataCloneError');
}

function cloneInto(value, memory) {
  if (typeof value === 'symbol') throw dataCloneError(String(value));
  if (typeof value === 'function') throw dataCloneError(`function ${value.name || ''}`.trim());
  if (value === null || typeof value !== 'object') return value;
  const seen = memory.get(value);
  if (seen !== undefined) return seen;

  const tag = toTag.call(value);
  let out;
  switch (tag) {
    case '[object Boolean]':
    case '[object Number]':
    case '[object String]':
    case '[object BigInt]':
      out = Object(value.valueOf());
      memory.set(value, out);
      return out;
    case '[object Date]':
      out = new Date(value.getTime());
      memory.set(value, out);
      return out;
    case '[object RegExp]':
      out = new RegExp(value.source, value.flags);
      memory.set(value, out);
      return out;
    case '[object ArrayBuffer]':
      out = value.slice(0);
      memory.set(value, out);
      return out;
    case '[object Map]':
      out = new Map();
      memory.set(value, out);
      for (const [k, v] of value) out.set(cloneInto(k, memory), cloneInto(v, memory));
      return out;
    case '[object Set]':
      out = new Set();
      memory.set(value, out);
      for (const v of value) out.add(cloneInto(v, memory));
      return out;
    case '[object Error]': {
      const name = value.name;
      const Ctor = ['EvalError', 'RangeError', 'ReferenceError', 'SyntaxError', 'TypeError',
        'URIError'].includes(name) ? globalThis[name] : Error;
      out = new Ctor(value.message);
      memory.set(value, out);
      if (typeof value.stack === 'string') out.stack = value.stack;
      return out;
    }
    case '[object Array]':
      out = new Array(value.length);
      break;
    case '[object Object]':
      out = {};
      break;
    default:
      if (ArrayBuffer.isView(value)) {
        const buffer = cloneInto(value.buffer, memory);
        out = tag === '[object DataView]'
          ? new DataView(buffer, value.byteOffset, value.byteLength)
          : new (getPrototypeOf(value).constructor)(buffer, value.byteOffset, value.length);
        memory.set(value, out);
        return out;
      }
      throw dataCloneError(tag.slice(8, -1));
  }
  memory.set(value, out);
  for (const key of ownKeys(value)) out[key] = cloneInto(value[key], memory);
  return out;
}

if (typeof globalThis.structuredClone !== 'function') {
  Object.defineProperty(globalThis, 'structuredClone', {
    value: function structuredClone(value) {
      if (arguments.length === 0) {
        throw new TypeError("Failed to execute 'structuredClone': 1 argument required, but only 0 present.");
      }
      return cloneInto(value, new Map());
    },
    writable: true,
    enumerable: false,
    configurable: true,
  });
}
