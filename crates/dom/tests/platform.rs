//! URL, URLSearchParams, TextEncoder, TextDecoder, atob, btoa, structuredClone,
//! navigator and customElements against their standards. Expected values are the
//! standards' own examples, or what WPT's url, encoding and html/webappapis tests
//! assert; none comes from our output.

mod common;

use common::limits;
use surimono_core::{Limits, SandboxError};
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

const PAGE: &str = "<!doctype html><html><head></head><body><iframe id=f></iframe></body></html>";

fn runtime_with(l: Limits) -> DomRuntime {
    let mut rt = DomRuntime::new(l).unwrap();
    let mut doc = Document::parse(PAGE).unwrap();
    doc.set_url("http://example.test/a/b.html?q=1");
    rt.load(doc);
    rt
}

fn run(script: &str) -> String {
    runtime_with(limits()).eval(script).unwrap()
}

/// `f()`'s exception as `name` or `DOMException name`, or `none`.
const CAUGHT: &str =
    "const thrown = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };";

fn caught(script: &str) -> String {
    run(&format!("{CAUGHT}{script}"))
}

#[test]
fn url_parts_follow_the_standard() {
    assert_eq!(
        run(
            r#"const u = new URL('https://u:p@EXAMPLE.org:8080/a/../b?x=1#f');
            [u.href, u.origin, u.protocol, u.username, u.password, u.host, u.hostname, u.port,
             u.pathname, u.search, u.hash].join(' ')"#
        ),
        "https://u:p@example.org:8080/b?x=1#f https://example.org:8080 https: u p \
         example.org:8080 example.org 8080 /b ?x=1 #f"
    );
    assert_eq!(
        run(
            r#"[new URL('../c', 'http://e.test/a/b/d').href, new URL('//x.test/', 'https://e.test').href,
             String(new URL('http://e.test')), JSON.stringify({ u: new URL('http://e.test') })].join(' ')"#
        ),
        r#"http://e.test/a/c https://x.test/ http://e.test/ {"u":"http://e.test/"}"#
    );
    assert_eq!(
        caught(
            r#"[thrown(() => new URL('nope')), thrown(() => new URL('/x', 'nope')),
                  thrown(() => URL('http://e.test')), URL.canParse('nope'), URL.canParse('/x', 'http://e.test'),
                  URL.parse('nope'), URL.parse('/x', 'http://e.test').href].join(' ')"#
        ),
        "TypeError TypeError TypeError false true  http://e.test/x"
    );
    assert_eq!(run("String(webkitURL === URL)"), "true");
}

#[test]
fn url_setters_follow_the_standard() {
    assert_eq!(
        run(r#"const u = new URL('http://e.test:81/p?q#h');
            u.protocol = 'https'; u.username = 'a b'; u.password = 'c'; u.host = 'x.test:99';
            u.port = ''; u.hostname = 'y.test'; u.pathname = '/a b'; u.search = 'k=v w'; u.hash = 'z';
            u.href"#),
        "https://a%20b:c@y.test/a%20b?k=v%20w#z"
    );
    assert_eq!(
        run(r#"const u = new URL('http://e.test/');
            u.protocol = 'nonsense:'; u.port = 'abc'; u.href"#),
        "http://e.test/"
    );
    assert_eq!(
        caught(
            r#"const u = new URL('http://e.test/');
            [thrown(() => { u.href = 'nope'; }), u.href].join(' ')"#
        ),
        "TypeError http://e.test/"
    );
}

#[test]
fn search_params_follow_the_standard() {
    assert_eq!(
        run(
            r#"const p = new URLSearchParams('?a=1&b=2&a=3&c&=d&&e=%F0%9F%98%80+x');
            [p.size, p.get('a'), p.getAll('a').join(), p.get('c'), p.get(''), p.get('e'), p.get('no'),
             p.has('a'), p.has('a', '3'), p.has('a', '9'), String(p)].join('|')"#
        ),
        "6|1|1,3||d|😀 x||true|true|false|a=1&b=2&a=3&c=&=d&e=%F0%9F%98%80+x"
    );
    assert_eq!(
        run(r#"const p = new URLSearchParams([['a', '1'], ['b', '2']]);
            const q = new URLSearchParams({ x: 1, y: 'z w' });
            const r = new URLSearchParams(p);
            r.append('c', 'é&=');
            [String(p), String(q), String(r), String(new URLSearchParams()), new URLSearchParams(undefined).size].join('|')"#),
        "a=1&b=2|x=1&y=z+w|a=1&b=2&c=%C3%A9%26%3D||0"
    );
    assert_eq!(
        caught(
            r#"[thrown(() => new URLSearchParams([['a']])), thrown(() => new URLSearchParams([1])),
               thrown(() => URLSearchParams()), thrown(() => new URLSearchParams().append('a'))].join()"#
        ),
        "TypeError,TypeError,TypeError,TypeError"
    );
    assert_eq!(
        run(
            r#"const p = new URLSearchParams('z=1&a=2&z=3&\u{1f308}=4&ﬁ=5');
            p.delete('z', '1'); p.set('a', '9'); p.set('n', '0'); p.append('a', 'x');
            p.set('a', 'y'); p.sort();
            [...p.keys()].join() + '|' + [...p.values()].join() + '|' + [...p].map(([k, v]) => k + v).join()"#
        ),
        "a,n,z,\u{1f308},\u{fb01}|y,0,3,4,5|ay,n0,z3,\u{1f308}4,\u{fb01}5"
    );
}

#[test]
fn search_params_iterate_live_as_webidl_says() {
    assert_eq!(
        run(r#"const p = new URLSearchParams('a=1&b=2&c=3');
            const log = [];
            p.forEach(function (v, k, o) {
              log.push(k + v + (o === p) + this.t);
              if (k === 'a') p.delete('b');
            }, { t: 7 });
            const i = p.entries();
            const proto = Object.getPrototypeOf(i);
            [log.join(), i.next().value.join(), Object.prototype.toString.call(i),
             p[Symbol.iterator] === p.entries, proto.next.call(i).value.join(), i.next().done].join('|')"#),
        "a1true7,c3true7|a,1|[object URLSearchParams Iterator]|true|c,3|true"
    );
}

#[test]
fn a_urls_params_and_its_query_stay_one() {
    assert_eq!(
        run(r#"const u = new URL('http://e.test/?a=1&b=2');
            const p = u.searchParams;
            const same = u.searchParams === p;
            p.append('c', 'x y');
            const one = u.href;
            p.delete('a'); p.delete('b'); p.delete('c');
            const two = u.href;
            u.search = '?m=n';
            const three = p.get('m');
            u.href = 'http://e.test/?o=p#h';
            [same, one, two, three, p.get('o'), [...p.keys()].join()].join(' ')"#),
        "true http://e.test/?a=1&b=2&c=x+y http://e.test/ n p o"
    );
}

#[test]
fn text_encoder_writes_utf8() {
    assert_eq!(
        run(r#"const e = new TextEncoder();
            const b = e.encode('aé€😀\ud800');
            [e.encoding, b instanceof Uint8Array, b.join(), e.encode().length].join(' ')"#),
        "utf-8 true 97,195,169,226,130,172,240,159,152,128,239,191,189 0"
    );
    assert_eq!(
        run(r#"const e = new TextEncoder();
            const d = new Uint8Array(5);
            const r = e.encodeInto('a€😀', d);
            const s = e.encodeInto('😀', new Uint8Array(3));
            [r.read, r.written, d.join(), s.read, s.written].join(' ')"#),
        "2 4 97,226,130,172,0 0 0"
    );
    assert_eq!(
        caught(
            r#"const e = new TextEncoder();
            [thrown(() => e.encodeInto('a', new Uint16Array(1))), thrown(() => e.encodeInto('a', [])),
             thrown(() => TextEncoder.prototype.encode.call({}))].join()"#
        ),
        "TypeError,TypeError,TypeError"
    );
}

#[test]
fn text_decoder_follows_the_encoding_standard() {
    assert_eq!(
        run(r#"const d = new TextDecoder();
            const u = (...b) => new Uint8Array(b);
            [d.encoding, d.fatal, d.ignoreBOM, d.decode(), d.decode(u(0xef, 0xbb, 0xbf, 0x61)),
             d.decode(u(0x61, 0xff, 0x62)), d.decode(u(0xf0, 0x9f, 0x98, 0x80).buffer),
             d.decode(new DataView(u(0, 0x61, 0).buffer, 1, 1)),
             d.decode(u(0, 0x61, 0).subarray(1, 2)),
             new TextDecoder('utf-8', { ignoreBOM: true }).decode(u(0xef, 0xbb, 0xbf)).length].join('|')"#),
        "utf-8|false|false||a|a\u{fffd}b|😀|a|a|1"
    );
    assert_eq!(
        run(r#"const d = new TextDecoder();
            const u = (...b) => new Uint8Array(b);
            [d.decode(u(0xe2, 0x82), { stream: true }), d.decode(u(0xac), { stream: true }),
             d.decode(u(0xf0), { stream: true }), d.decode()].join('|')"#),
        "|€||\u{fffd}"
    );
    assert_eq!(
        run(r#"const u = (...b) => new Uint8Array(b);
            [new TextDecoder('ascii').encoding, new TextDecoder(' UTF8 ').encoding,
             new TextDecoder('sjis').decode(u(0x82, 0xa0)), new TextDecoder('euc-jp').decode(u(0xa4, 0xa2)),
             new TextDecoder('utf-16le').decode(u(0x42, 0x30)),
             new TextDecoder('utf-16be').decode(u(0x30, 0x42)),
             new TextDecoder('windows-1252').decode(u(0x80))].join('|')"#),
        "windows-1252|utf-8|あ|あ|あ|あ|€"
    );
    assert_eq!(
        caught(
            r#"const f = new TextDecoder('utf-8', { fatal: true });
            const u = (...b) => new Uint8Array(b);
            [thrown(() => new TextDecoder('nope')), thrown(() => new TextDecoder('replacement')),
             thrown(() => new TextDecoder('hz-gb-2312')), thrown(() => f.decode(u(0xff))),
             thrown(() => f.decode(u(0xff), { stream: true })), f.decode(u(0x61)),
             thrown(() => f.decode(1)), thrown(() => f.decode('a'))].join()"#
        ),
        "RangeError,RangeError,RangeError,TypeError,TypeError,a,TypeError,TypeError"
    );
}

#[test]
fn base64_is_on_the_window() {
    assert_eq!(
        run(
            r#"[btoa('héllo'.replace('é', '\xe9')), atob('aOlsbG8='), atob(' aG k\n='), btoa(''), atob(''),
             btoa(null), atob(null).length].join('|')"#
        ),
        "aOlsbG8=|h\u{e9}llo|hi|||bnVsbA==|3"
    );
    assert_eq!(
        caught(
            r#"const dom = (f) => { try { f(); return 'none'; } catch (e) { return e instanceof DOMException && e.name; } };
            [dom(() => btoa('あ')), dom(() => atob('a')), dom(() => atob('a!==')), thrown(() => atob())].join()"#
        ),
        "InvalidCharacterError,InvalidCharacterError,InvalidCharacterError,TypeError"
    );
}

#[test]
fn structured_clone_keeps_what_the_standard_keeps() {
    assert_eq!(
        run(
            r#"const a = { n: 1, s: 'x', u: undefined, nul: null, big: 5n, nested: [1, { y: 2 }], d: new Date(5) };
            a.self = a;
            const c = structuredClone(a);
            [c !== a, c.self === c, c.n, c.s, 'u' in c, c.nul, c.big, c.nested[1].y, c.nested !== a.nested,
             c.d instanceof Date, c.d.getTime(), c.d !== a.d].join()"#
        ),
        "true,true,1,x,true,,5,2,true,true,5,true"
    );
    assert_eq!(
        run(r#"const m = new Map([[1, { a: 1 }]]), s = new Set(['x']);
            const shared = new Date(1), r = /a/gi;
            const c = structuredClone({ m, s, p: shared, q: shared, r, b: new Number(3), t: new String('s'), f: new Boolean(false) });
            [c.m.get(1).a, c.s.has('x'), c.p === c.q, c.r.source, c.r.flags, typeof c.b, c.b + 1, c.t + 't', c.f.valueOf()].join()"#),
        "1,true,true,a,gi,object,4,st,false"
    );
    assert_eq!(
        run(r#"const b = new Uint8Array([1, 2, 3, 4]);
            const v = new DataView(b.buffer, 1, 2), w = new Int16Array(b.buffer, 2, 1);
            const c = structuredClone({ b, v, w, raw: b.buffer });
            c.b[1] = 9;
            [c.b.join(), b[1], c.b.buffer === c.raw, c.v.buffer === c.raw, c.v.byteOffset, c.v.byteLength,
             c.w.length, c.w.byteOffset, c.raw !== b.buffer].join()"#),
        "1,9,3,4,2,true,true,1,2,1,2,true"
    );
    assert_eq!(
        run(r#"const e = new RangeError('m');
            const x = new (class Own extends Error {})('o');
            x.name = 'Own';
            const c = structuredClone({ e, x, plain: Object.assign(new Error('p', { cause: 7 }), { extra: 1 }) });
            [c.e instanceof RangeError, c.e.message, c.e.name, c.x.name, c.x.message, 'extra' in c.plain,
             c.plain.cause].join()"#),
        "true,m,RangeError,Error,o,false,7"
    );
    assert_eq!(
        run(
            r#"class P { constructor() { this.a = 1; } get g() { return 2; } }
            const c = structuredClone(new P());
            const proto = Object.getPrototypeOf(c) === Object.prototype;
            let order = '';
            const o = { get x() { order += 'x'; delete this.y; return 1; }, y: 2 };
            const d = structuredClone(o);
            [proto, c.a, c.g, order, 'y' in d].join()"#
        ),
        "true,1,,x,false"
    );
}

#[test]
fn structured_clone_refuses_what_the_standard_refuses() {
    assert_eq!(
        caught(
            r#"const dce = (f) => { try { f(); return 'none'; } catch (e) { return e instanceof DOMException && e.name; } };
            [() => structuredClone(() => {}), () => structuredClone(Symbol()), () => structuredClone({ a: Symbol() }),
             () => structuredClone(new Proxy({}, {})), () => structuredClone(new Proxy([], {})),
             () => structuredClone(Promise.resolve()), () => structuredClone(new WeakMap()),
             () => structuredClone(new WeakSet()), () => structuredClone(Object(Symbol())),
             () => structuredClone(document), () => structuredClone(document.body),
             () => structuredClone(new URL('http://e.test'))].map(dce).join()"#
        ),
        vec!["DataCloneError"; 12].join(",")
    );
    assert_eq!(caught("thrown(() => structuredClone())"), "TypeError");
}

#[test]
fn structured_clone_transfers_buffers() {
    assert_eq!(
        run(r#"const b = new Uint8Array([1, 2]).buffer;
            const c = structuredClone({ b, v: new Uint8Array(b) }, { transfer: [b] });
            [b.byteLength, b.detached, c.b.byteLength, c.v[1], c.v.buffer === c.b].join()"#),
        "0,true,2,2,true"
    );
    assert_eq!(
        caught(
            r#"const b = new ArrayBuffer(1);
            const dce = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
            [dce(() => structuredClone(1, { transfer: [b, b] })), dce(() => structuredClone(1, { transfer: [{}] })),
             b.byteLength, dce(() => structuredClone(() => {}, { transfer: [b] })), b.byteLength].join()"#
        ),
        "DataCloneError,DataCloneError,1,DataCloneError,1"
    );
}

#[test]
fn navigator_has_fixed_values() {
    assert_eq!(
        run(r#"const n = navigator;
            [n === window.navigator, n === clientInformation, n instanceof Navigator, n.appCodeName, n.appName,
             n.product, n.productSub, n.vendor, n.vendorSub, n.platform, n.userAgent, n.appVersion, n.language,
             n.languages.join(), Object.isFrozen(n.languages), n.languages === n.languages, n.onLine,
             n.cookieEnabled, n.hardwareConcurrency, n.maxTouchPoints, n.pdfViewerEnabled, n.javaEnabled(),
             n.plugins.length, n.mimeTypes.length, n.plugins.item(0), n.mimeTypes.namedItem('x'),
             frames[0].navigator === n].join('|')"#),
        "true|true|true|Mozilla|Netscape|Gecko|20030107|||\
         |Mozilla/5.0 (compatible; surimono)|5.0 (compatible; surimono)|en-US|en-US|true|true|false|\
         false|1|0|false|false|0|0|||true"
    );
    assert_eq!(
        caught("thrown(() => Object.create(Navigator.prototype).userAgent)"),
        "TypeError"
    );
}

#[test]
fn custom_elements_is_a_registry() {
    assert_eq!(
        run(r#"class A extends HTMLElement {}
            class B extends HTMLElement {}
            const order = [];
            customElements.whenDefined('x-a').then((c) => order.push(c === A));
            customElements.define('x-a', A);
            [customElements.get('x-a') === A, customElements.get('x-b'), customElements.getName(A),
             customElements.getName(B), customElements instanceof CustomElementRegistry].join()"#),
        "true,,x-a,,true"
    );
    assert_eq!(
        caught(
            r#"const dom = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
            class A extends HTMLElement {}
            customElements.define('x-a', A);
            [dom(() => customElements.define('x-a', class extends HTMLElement {})),
             dom(() => customElements.define('x-b', A)),
             dom(() => customElements.define('nohyphen', class {})),
             dom(() => customElements.define('X-c', class {})),
             dom(() => customElements.define('font-face', class {})),
             dom(() => customElements.define('x-d', 1)),
             dom(() => customElements.upgrade(document.body))].join()"#
        ),
        "NotSupportedError,NotSupportedError,SyntaxError,SyntaxError,SyntaxError,TypeError,none"
    );
}

/// A script that replaces what the glue could look up gets the same answers.
#[test]
fn replacing_built_ins_changes_nothing() {
    assert_eq!(
        run(r#"
            const u = new URL('http://e.test/?a=1'), p = u.searchParams, d = new TextDecoder();
            const bytes = new Uint8Array([97]);
            const box = { m: new Map([[1, 2]]) };
            Object.defineProperty(Array.prototype, 0, { set() { throw 'array'; }, configurable: true });
            Object.defineProperty(Object.prototype, 'k', { set() { throw 'object'; }, get() { throw 'object'; }, configurable: true });
            Object.defineProperty(Object.prototype, 'value', { set() { throw 'object'; }, configurable: true });
            String.prototype.indexOf = String.prototype.slice = () => { throw 'string'; };
            String.fromCharCode = Map.prototype.get = Map.prototype.set = () => { throw 'map'; };
            Reflect.apply = Object.create = Object.keys = () => { throw 'reflect'; };
            Uint8Array = class { constructor() { throw 'u8'; } };
            ArrayBuffer.prototype.slice = () => { throw 'buffer'; };
            delete Object.prototype.value;
            delete Object.prototype.k;
            delete Array.prototype[0];
            p.append('b', '2'); p.sort();
            const c = structuredClone(box);
            [u.href, new URLSearchParams('x=y').get('x'), d.decode(bytes), new TextEncoder().encode('a')[0],
             btoa('a'), atob('YQ=='), c.m.size].join(' ')"#),
        "http://e.test/?a=1&b=2 y a 97 YQ== a 1"
    );
}

/// Each is linear in a string a script made, and ends with the limit, typed.
#[test]
fn a_loop_of_each_ends_with_a_limit() {
    let tiny = || {
        common::scaled_limits(Limits::default().with_time(std::time::Duration::from_millis(200)))
    };
    for script in [
        "const s = 'a'.repeat(1 << 20); for (;;) atob(btoa(s));",
        "for (;;) new URL('http://e.test/' + 'a'.repeat(1 << 16));",
        "const u = new URL('http://e.test/'); for (;;) u.search = 'a=' + 'b'.repeat(1 << 16);",
        "const p = new URLSearchParams('a=b&'.repeat(1 << 12)); for (;;) p.sort();",
        "const b = new Uint8Array(1 << 18); const d = new TextDecoder(); for (;;) d.decode(b);",
        "const e = new TextEncoder(); const s = 'é'.repeat(1 << 18); for (;;) e.encode(s);",
        "const o = { a: [1, 2, 3] }; for (;;) structuredClone(o);",
    ] {
        let mut rt = runtime_with(tiny());
        let e = rt.eval::<()>(script).unwrap_err();
        assert!(
            matches!(e, DomError::Sandbox(SandboxError::TimeLimit)),
            "{script}: {e:?}"
        );
    }
}

/// What the script gets back stays in the JavaScript heap.
#[test]
fn encoded_bytes_count_against_the_memory_limit() {
    let mut rt = runtime_with(Limits::default().with_memory_bytes(16 * 1024 * 1024));
    let e = rt
        .eval::<()>(
            "const e = new TextEncoder(); const s = 'a'.repeat(1 << 20); const keep = [];
             for (;;) keep.push(e.encode(s));",
        )
        .unwrap_err();
    assert!(
        matches!(e, DomError::Sandbox(SandboxError::MemoryLimit)),
        "{e:?}"
    );
}

#[test]
fn deep_clones_end_in_a_stack_limit() {
    let mut rt = runtime_with(limits());
    let e = rt
        .eval::<()>("let o = {}; for (let i = 0; i < 2e5; i++) o = { o }; structuredClone(o);")
        .unwrap_err();
    assert!(
        matches!(e, DomError::Sandbox(SandboxError::StackOverflow)),
        "{e:?}"
    );
}

#[test]
fn a_url_past_the_cap_is_refused() {
    let mut rt = runtime_with(limits());
    let got: String = rt
        .eval(
            "const long = 'http://e.test/' + 'a'.repeat(3 << 20);
             const u = new URL('http://e.test/');
             const names = [];
             try { new URL(long); } catch (e) { names.push(e.name); }
             try { u.hash = 'a'.repeat(3 << 20); } catch (e) { names.push(e.name); }
             names.push(URL.canParse(long), URL.parse(long), u.href);
             names.join()",
        )
        .unwrap();
    assert_eq!(got, "TypeError,TypeError,false,,http://e.test/");
}

/// Every open stream is charged, so a script can't grow them outside the heap
/// limit; a stream's end gives the charge back, so ones that finish cost nothing.
#[test]
fn open_decoders_are_charged() {
    let mut l = DomLimits::default();
    l.max_text_bytes = 1024 * 1024;
    let doc = Document::parse_with_limits(PAGE, l).unwrap();
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(doc);
    let got: String = rt
        .eval(
            "const b = new Uint8Array([0xe2]);
             for (let i = 0; i < 2000; i++) {
               const d = new TextDecoder();
               d.decode(b, { stream: true });
               d.decode();
             }
             'finished'",
        )
        .unwrap();
    assert_eq!(got, "finished");
    let e = rt
        .eval::<()>(
            "const keep = [];
             for (;;) { const d = new TextDecoder(); d.decode(b, { stream: true }); keep.push(d); }",
        )
        .unwrap_err();
    assert!(matches!(e, DomError::TextLimit { .. }), "{e:?}");
}

/// The IDNA code is the one part of a URL's cost that isn't plainly linear:
/// Punycode looks like it could be, so each shape runs at the size the cap allows.
#[test]
fn hostile_hosts_cost_little() {
    for name in [
        "'é'.repeat(1 << 19)",
        "'aé'.repeat(1 << 19 >> 1)",
        "'é.'.repeat(1 << 18)",
        "'xn--' + 'a'.repeat(1 << 20)",
        "'xn--a.'.repeat(1 << 17)",
        "'１'.repeat(1 << 18)",
    ] {
        let began = std::time::Instant::now();
        let mut rt = runtime_with(limits());
        let () = rt
            .eval(&format!(
                "const name = {name};
                 URL.canParse('http://' + name + '/');
                 const u = new URL('http://e.test/');
                 try {{ u.host = name; }} catch (e) {{}}
                 undefined"
            ))
            .unwrap();
        assert!(
            began.elapsed() < common::scaled(std::time::Duration::from_secs(10)),
            "{name}: {:?}",
            began.elapsed()
        );
    }
}

#[test]
fn nothing_here_varies_between_runs() {
    const SCRIPT: &str = r#"JSON.stringify([navigator.userAgent, navigator.platform, navigator.language,
        navigator.hardwareConcurrency, new URL('http://ÉXAMPLE.test/あ?い#う').href,
        btoa('x'), structuredClone({ a: 1 })])"#;
    assert_eq!(run(SCRIPT), run(SCRIPT));
}

#[test]
fn the_memory_stays_with_the_render() {
    let mut rt = runtime_with(limits());
    let () = rt
        .eval("new TextDecoder().decode(new Uint8Array([0xe2]), { stream: true }); undefined")
        .unwrap();
    let doc = rt.take().unwrap();
    doc.check_consistency().unwrap();
}
