//! matchMedia, requestIdleCallback, the CSS namespace and crypto (P2-DOM-20,
//! part one). Expected values are Chromium 153's, from probes run in it for
//! this task (the cases are listed for `wpt/probes`), or the standards' where
//! we follow the standard instead; none comes from our output. Where we
//! depart from Chromium, the test says why.

mod common;

use common::limits;
use surimono_core::{SandboxOptions, SettleOptions};
use surimono_dom::{Document, DomError, DomRuntime, LayoutPolicy};

const HEAD: &str = "<!doctype html><html><head><script>var L=[];function log(){L.push([].map.call(arguments,String).join(' '))}</script></head><body>";

fn runtime(url: Option<&str>, seed: u64) -> DomRuntime {
    let options = SandboxOptions::default()
        .with_limits(limits())
        .with_random_seed(seed);
    let mut rt = DomRuntime::with_options(options).unwrap();
    let mut doc = Document::parse(&format!("{HEAD}</body></html>")).unwrap();
    if let Some(u) = url {
        doc.set_url(u);
    }
    rt.load(doc);
    rt
}

fn run(script: &str) -> String {
    runtime(None, 0).eval(script).unwrap()
}

/// Runs `body` as a page and returns what it logged, one entry a line.
fn page_at(url: Option<&str>, seed: u64, body: &str, settle: &SettleOptions) -> String {
    let mut rt = DomRuntime::with_options(
        SandboxOptions::default()
            .with_limits(limits())
            .with_random_seed(seed),
    )
    .unwrap();
    let mut doc = Document::parse(&format!("{HEAD}{body}</body></html>")).unwrap();
    if let Some(u) = url {
        doc.set_url(u);
    }
    rt.load(doc);
    let report = rt.run_page(settle, |_| None).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    rt.eval("L.join('\\n')").unwrap()
}

/// `run` with time for a million calls on a debug build: what is bounded
/// here is the work, which a time limit would only hide.
fn long(script: &str) -> String {
    let options = SandboxOptions::default()
        .with_limits(limits().with_time(common::scaled(std::time::Duration::from_secs(300))));
    let mut rt = DomRuntime::with_options(options).unwrap();
    rt.load(Document::parse(&format!("{HEAD}</body></html>")).unwrap());
    let started = std::time::Instant::now();
    let r = rt.eval(script).unwrap();
    eprintln!("{:?}", started.elapsed());
    r
}

fn page(body: &str) -> String {
    page_at(None, 0, body, &SettleOptions::default())
}

#[test]
fn the_globals_have_chromiums_shape() {
    // Chromium 153: each a property of the window itself, enumerable,
    // writable and configurable for an operation; a getter for an attribute.
    let shape = |name: &str| {
        format!(
            "(() => {{ const d = Object.getOwnPropertyDescriptor(window, '{name}') || {{}};
              const o = window['{name}'];
              return ['{name}', typeof o, o && o.name, o && o.length, d.enumerable, d.writable,
                      d.configurable, 'get' in d].join(' '); }})()"
        )
    };
    let cases = [
        (
            "matchMedia",
            "matchMedia function matchMedia 1 true true true false",
        ),
        (
            "requestIdleCallback",
            "requestIdleCallback function requestIdleCallback 1 true true true false",
        ),
        (
            "cancelIdleCallback",
            "cancelIdleCallback function cancelIdleCallback 1 true true true false",
        ),
        ("CSS", "CSS object   false true true false"),
        ("crypto", "crypto object   true  true true"),
        ("Crypto", "Crypto function Crypto 0 false true true false"),
        (
            "IdleDeadline",
            "IdleDeadline function IdleDeadline 0 false true true false",
        ),
        (
            "MediaQueryList",
            "MediaQueryList function MediaQueryList 0 false true true false",
        ),
        (
            "MediaQueryListEvent",
            "MediaQueryListEvent function MediaQueryListEvent 1 false true true false",
        ),
        (
            "isSecureContext",
            "isSecureContext boolean   true  true true",
        ),
    ];
    for (name, want) in cases {
        assert_eq!(run(&shape(name)), want, "{name}");
    }
    assert_eq!(
        run(
            "[Object.prototype.toString.call(CSS), Object.getPrototypeOf(CSS) === Object.prototype,
              String(crypto), crypto === crypto, Object.getPrototypeOf(crypto) === Crypto.prototype,
              Object.getPrototypeOf(MediaQueryList.prototype) === EventTarget.prototype,
              Object.prototype.toString.call(matchMedia('screen'))].join(' ')"
        ),
        "[object CSS] true [object Crypto] true true true [object MediaQueryList]"
    );
    for (name, len) in [("escape", 1), ("supports", 1)] {
        assert_eq!(
            run(&format!(
                "const d = Object.getOwnPropertyDescriptor(CSS, '{name}');
                 [typeof d.value, d.value.length, d.enumerable, d.writable, d.configurable].join()"
            )),
            format!("function,{len},true,true,true")
        );
    }
    // Illegal constructors and missing arguments, as TypeErrors.
    assert_eq!(
        run("const t = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
             [t(() => new Crypto()), t(() => new IdleDeadline()), t(() => new MediaQueryList()),
              t(() => matchMedia()), t(() => requestIdleCallback()), t(() => requestIdleCallback(1)),
              t(() => cancelIdleCallback()), t(() => CSS.escape()), t(() => CSS.supports()),
              t(() => crypto.getRandomValues()),
              t(() => Crypto.prototype.getRandomValues.call({}, new Uint8Array(1)))].join(' ')"),
        "TypeError TypeError TypeError TypeError TypeError TypeError TypeError TypeError \
         TypeError TypeError TypeError"
    );
}

#[test]
fn match_media_answers_for_the_window_scripts_see() {
    // innerWidth and innerHeight are 0 until layout gives them (glue_layout.js),
    // so `width` is 0: max-* matches, min-* doesn't. MQ4 for the rest, and
    // `screen` while scripts run.
    assert_eq!(
        run("[innerWidth, innerHeight, devicePixelRatio].concat(
               ['screen', 'print', 'all', '(max-width: 600px)', '(min-width: 1px)', '(width)',
                '(orientation: portrait)', '(hover: none)', '(pointer: fine)',
                '(prefers-color-scheme: light)', '(prefers-reduced-motion: no-preference)',
                '(resolution: 1dppx)', '(color)', '(foo)', 'not (foo)', 'nonsense!!']
               .map(q => { const m = matchMedia(q); return m.media + '=' + m.matches; })).join('|')"),
        "0|0|1|screen=true|print=false|all=true|(max-width: 600px)=true|(min-width: 1px)=false|\
         (width)=false|(orientation: portrait)=true|(hover: none)=true|(pointer: fine)=false|\
         (prefers-color-scheme: light)=true|(prefers-reduced-motion: no-preference)=true|\
         (resolution: 1dppx)=true|(color)=true|(foo)=false|not (foo)=false|not all=false"
    );
    // A new list each call; the text as given, converted.
    assert_eq!(
        run("const a = matchMedia('screen'), b = matchMedia('screen');
             [a === b, matchMedia(null).media, matchMedia(undefined).media, matchMedia(1).media,
              matchMedia('').media === '', matchMedia('').matches].join(' ')"),
        "false null undefined not all true true"
    );
}

#[test]
fn match_media_listeners_wait_and_never_hear() {
    // Nothing changes during a render: listeners are kept, `change` doesn't
    // come. A page's own dispatch reaches them, as in a browser.
    assert_eq!(
        page(
            "<script>var m = matchMedia('(min-width: 500px)'), n = 0;
             m.addListener(() => n++); m.addListener(null); m.removeListener(null);
             m.onchange = () => n++;
             m.addEventListener('change', () => n++);
             setTimeout(() => log('timer', n, m.onchange !== null), 1000);
             m.dispatchEvent(new MediaQueryListEvent('change', { media: m.media, matches: true }));
             log('dispatched', n);
             var e = new MediaQueryListEvent('change'); log(e.media === '', e.matches);</script>"
        ),
        "dispatched 3\ntrue false\ntimer 3 true"
    );
}

#[test]
fn match_media_on_size_is_a_layout_query() {
    let mut rt = runtime(None, 0);
    rt.set_layout_policy(LayoutPolicy::Throw);
    // The text and a feature with no size are no layout query.
    assert_eq!(
        rt.eval::<String>("const m = matchMedia('(min-width: 1px)'); m.media + ' ' + matchMedia('(color)').matches")
            .unwrap(),
        "(min-width: 1px) true"
    );
    let err = rt
        .eval::<bool>("matchMedia('(min-width: 1px)').matches")
        .unwrap_err();
    assert!(matches!(err, DomError::LayoutQuery { .. }), "{err:?}");
}

#[test]
fn idle_callbacks_run_in_idle_periods() {
    // requestIdleCallback §5: after the tasks due now (the page's life, a
    // zero timer), in a period that ends at the next task or 50 ms on; a
    // callback queued in a period waits for the next. Chromium runs a frame
    // first (`raf` before `idle1`), and its timer at 101 before the idle
    // callback set at 100: the spec lets an idle period come before the next
    // frame or timer, with its deadline there. `load` comes before the zero
    // timer, as the page's life is queued first (DESIGN.md).
    let log = page(
        "<script>
         requestIdleCallback(d => log('idle1', d.didTimeout, d.timeRemaining()));
         setTimeout(() => log('t0'), 0);
         setTimeout(() => log('t10'), 10);
         requestAnimationFrame(() => log('raf'));
         window.addEventListener('load', () => log('load'));
         var gone = requestIdleCallback(() => log('cancelled')); cancelIdleCallback(gone);
         var a = setTimeout(() => {}, 0), b = requestIdleCallback(() => {});
         log('ids', gone, b);
         requestIdleCallback(function (d) {
           log('this', this === window, arguments.length, Object.prototype.toString.call(d),
               d instanceof IdleDeadline, d.timeRemaining());
           requestIdleCallback(d => log('nested', performance.now(), d.timeRemaining()));
           setTimeout(() => log('t-in-idle'), 0);
         });
         setTimeout(() => requestIdleCallback(d => log('idle-from-t100', performance.now(), d.timeRemaining())), 100);
         setTimeout(() => log('t101'), 101);
         </script>",
    );
    assert_eq!(
        log,
        "ids 2 3\nload\nt0\nidle1 false 10\nthis true 1 [object IdleDeadline] true 10\nt-in-idle\n\
         t10\nnested 10 6\nraf\nidle-from-t100 100 1\nt101"
    );
}

#[test]
fn an_idle_timeout_runs_a_callback_no_period_reached() {
    // §5.5: queued during a period, so it waits for the next at 50; its
    // timeout of 5 comes first, with didTimeout and nothing remaining.
    assert_eq!(
        page(
            "<script>requestIdleCallback(() => {
               requestIdleCallback(d => log(performance.now(), d.didTimeout, d.timeRemaining()), { timeout: 5 });
             });</script>"
        ),
        "5 true 0"
    );
    // WebIDL `unsigned long`: Infinity, NaN and 0 are no timeout, -1 is
    // 2^32 - 1; a non-object is a TypeError, null the empty dictionary.
    assert_eq!(
        page(
            "<script>for (const v of [Infinity, NaN, -1, 0, 1e10, '5', null]) {
               requestIdleCallback(d => log('conv', String(v), d.didTimeout), { timeout: v });
             }
             try { requestIdleCallback(() => {}, 5) } catch (e) { log('opt5', e.name) }
             requestIdleCallback(() => {}, { get timeout() { log('getter'); return 1 } });
             cancelIdleCallback('x'); cancelIdleCallback(-1); log('cancel ok');</script>"
        ),
        "opt5 TypeError\ngetter\ncancel ok\nconv Infinity false\nconv NaN false\nconv -1 false\n\
         conv 0 false\nconv 10000000000 false\nconv 5 false\nconv null false"
    );
}

#[test]
fn an_idle_callback_that_throws_is_reported_and_the_next_runs() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse(&format!(
            "{HEAD}<script>window.onerror = (m) => log('onerror', m);
             requestIdleCallback(() => {{ throw new Error('boom') }});
             requestIdleCallback(() => log('after throw'));</script>"
        ))
        .unwrap(),
    );
    let report = rt.run_page(&SettleOptions::default(), |_| None).unwrap();
    assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
    assert_eq!(
        rt.eval::<String>("L.join('|')").unwrap(),
        "onerror Error: boom|after throw"
    );
}

#[test]
fn rearming_idle_callbacks_end_as_timers_do() {
    // Each period moves the clock 50 ms: the page settles at the virtual
    // budget, as with an interval nobody clears.
    let settle = SettleOptions {
        virtual_budget_ms: 1_000,
        ..SettleOptions::default()
    };
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse(&format!(
            "{HEAD}<script>var n = 0; (function f() {{ n++; requestIdleCallback(f); }})();</script>"
        ))
        .unwrap(),
    );
    let report = rt.run_page(&settle, |_| None).unwrap();
    assert!(report.settle.settled_by_budget(), "{report:?}");
    // Once from the script, then a period every 50 ms from 0 to 1,000.
    assert_eq!(rt.eval::<i32>("n").unwrap(), 22);
    // Ninety thousand at once run in one period, each a task (with time for
    // a debug build).
    let mut rt =
        DomRuntime::new(limits().with_time(common::scaled(std::time::Duration::from_secs(120))))
            .unwrap();
    rt.load(
        Document::parse(&format!(
            "{HEAD}<script>var n = 0; for (let i = 0; i < 9e4; i++) requestIdleCallback(() => n++);</script>"
        ))
        .unwrap(),
    );
    let report = rt.run_page(&SettleOptions::default(), |_| None).unwrap();
    assert_eq!(rt.eval::<i32>("n").unwrap(), 90_000, "{report:?}");
    assert_eq!(report.settle.virtual_now_ms, 0);
    // A million live: past the cap idle callbacks share with timers (100,000,
    // outside the JavaScript heap) the page ends with the memory limit.
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse(&format!(
            "{HEAD}<script>for (let i = 0; i < 1e6; i++) requestIdleCallback(() => {{}});</script>"
        ))
        .unwrap(),
    );
    let err = rt
        .run_page(&SettleOptions::default(), |_| None)
        .unwrap_err();
    assert!(
        matches!(
            err,
            DomError::Sandbox(surimono_core::SandboxError::MemoryLimit)
        ),
        "{err:?}"
    );
}

#[test]
fn css_escape_serializes_an_identifier() {
    // Chromium 153's answers, which are CSSOM §6.1's.
    assert_eq!(
        run(
            r#"const hex = (s) => [...s].map(c => c.length > 1 || c.charCodeAt(0) >= 0x80 || c.charCodeAt(0) < 0x20
                 ? 'U+' + c.codePointAt(0).toString(16) : c).join('');
              ['', 'a', '-', '--', '-a', '-1', '1', '1a', 'a1', '--1', '\0', 'a\0b',
              '\x01\x1f\x7f', '\x80', '\xe9', '\u{1f600}', '\ud800', '\udc00', 'a b', 'a!b', '#', '.', '\\',
              '_', '-_', ' ', '\t', '\n', 'a\x10', '~', '-\0', '0', '-0']
              .map(s => hex(CSS.escape(s))).join('|')
              + ' ' + [CSS.escape(1), CSS.escape(null), CSS.escape(undefined)].join(' ')"#
        ),
        r#"|a|\-|--|-a|-\31 |\31 |\31 a|a1|--1|U+fffd|aU+fffdb|\1 \1f \7f |U+80|U+e9|U+1f600|U+d800|U+dc00|a\ b|a\!b|\#|\.|\\|_|-_|\ |\9 |\a |a\10 |\~|-U+fffd|\30 |-\30  \31  null undefined"#
    );
}

#[test]
fn css_supports_answers_from_the_printers_style_engine() {
    // Where Chromium and stylo agree. Chromium says true for `fill` and
    // `backdrop-filter`, which stylo's Servo set doesn't have.
    assert_eq!(
        run(
            "[CSS.supports('color', 'red'), CSS.supports('display', 'grid'),
              CSS.supports('display', 'foo'), CSS.supports('--x', 'anything'),
              CSS.supports('color', 'red', 'extra'), CSS.supports('color'),
              CSS.supports('(display: grid) and (color: red)'), CSS.supports('display: flex'),
              CSS.supports('not (foo: bar)'), CSS.supports('selector(a > b)'),
              CSS.supports(null, 'x'), CSS.supports(undefined)].join(' ')"
        ),
        "true true false true true false true true true true false false"
    );
}

#[test]
fn get_random_values_checks_as_webcrypto_says() {
    assert_eq!(
        run("const t = (f) => { try { f(); return 'none'; } catch (e) { return e.name + (e.code ? '/' + e.code : ''); } };
             ['Int8Array','Uint8Array','Uint8ClampedArray','Int16Array','Uint16Array','Int32Array',
              'Uint32Array','BigInt64Array','BigUint64Array','Float32Array','Float64Array']
               .map(n => n + ':' + t(() => { const a = new window[n](4); if (crypto.getRandomValues(a) !== a) throw 1; }))
             .concat([
               t(() => crypto.getRandomValues(new DataView(new ArrayBuffer(4)))),
               t(() => crypto.getRandomValues(new Uint8Array(65536))),
               t(() => crypto.getRandomValues(new Uint8Array(65537))),
               t(() => crypto.getRandomValues(new Uint32Array(16385))),
               t(() => crypto.getRandomValues(new Float32Array(100000))),
               t(() => crypto.getRandomValues(null)), t(() => crypto.getRandomValues({})),
               t(() => crypto.getRandomValues([1])), t(() => crypto.getRandomValues(new ArrayBuffer(4))),
               t(() => crypto.getRandomValues(new Uint8Array(new ArrayBuffer(8, { maxByteLength: 16 })))),
               t(() => crypto.getRandomValues(new Uint8Array(new SharedArrayBuffer(8)))),
             ]).join(' ')"),
        "Int8Array:none Uint8Array:none Uint8ClampedArray:none Int16Array:none Uint16Array:none \
         Int32Array:none Uint32Array:none BigInt64Array:none BigUint64Array:none \
         Float32Array:TypeMismatchError/17 Float64Array:TypeMismatchError/17 TypeMismatchError/17 \
         none QuotaExceededError/22 QuotaExceededError/22 TypeMismatchError/17 TypeError TypeError \
         TypeError TypeError TypeError TypeError"
    );
    // A view's own bytes only; a detached one is returned as it is.
    assert_eq!(
        run("const sub = new Uint8Array(new ArrayBuffer(16), 4, 8); crypto.getRandomValues(sub);
             const b = new ArrayBuffer(8), u = new Uint8Array(b); structuredClone(b, { transfer: [b] });
             [new Uint8Array(sub.buffer, 0, 4).join(), new Uint8Array(sub.buffer, 12, 4).join(),
              sub.some(x => x !== 0), crypto.getRandomValues(u) === u, u.length,
              crypto.getRandomValues(new Uint8Array(0)).length].join(' ')"),
        "0,0,0,0 0,0,0,0 true true 0 0"
    );
}

#[test]
fn quota_exceeded_error_is_its_own_interface() {
    // WebIDL §2.8.1.1, as Chromium 153 has it: getRandomValues throws one with
    // no quota and no request.
    assert_eq!(
        run("let e; try { crypto.getRandomValues(new Uint8Array(65537)) } catch (x) { e = x }
             const t = (f) => { try { f(); return 'none'; } catch (x) { return x.name; } };
             const q = new QuotaExceededError('m', { quota: 1, requested: 2 });
             [e.constructor === QuotaExceededError, e instanceof DOMException, e.name, e.code, e.quota,
              e.requested, String(Object.getPrototypeOf(QuotaExceededError.prototype) === DOMException.prototype),
              q.message, q.quota, q.requested, QuotaExceededError.length,
              Object.prototype.toString.call(q), new QuotaExceededError().message === '',
              t(() => new QuotaExceededError('', { quota: -1 })),
              t(() => new QuotaExceededError('', { quota: 2, requested: 1 })),
              t(() => new QuotaExceededError('', 5)), t(() => QuotaExceededError()),
              t(() => Object.getOwnPropertyDescriptor(QuotaExceededError.prototype, 'quota').get.call({})),
              Object.getOwnPropertyDescriptor(window, 'QuotaExceededError').enumerable].join(' ')"),
        "true true QuotaExceededError 22   true m 1 2 0 [object QuotaExceededError] true RangeError \
         RangeError TypeError TypeError TypeError false"
    );
}

#[test]
fn random_values_come_from_the_seed() {
    let draw = "const a = new Uint32Array(8); crypto.getRandomValues(a); a.join() + ' ' + crypto.randomUUID()";
    let one = runtime(None, 7).eval::<String>(draw).unwrap();
    assert_eq!(one, runtime(None, 7).eval::<String>(draw).unwrap());
    assert_ne!(one, runtime(None, 8).eval::<String>(draw).unwrap());
    // Math.random's numbers don't move when crypto draws.
    let plain = runtime(None, 7)
        .eval::<String>("[Math.random(), Math.random()].join()")
        .unwrap();
    let after = runtime(None, 7)
        .eval::<String>(
            "crypto.getRandomValues(new Uint8Array(1000)); [Math.random(), Math.random()].join()",
        )
        .unwrap();
    assert_eq!(plain, after);
    // RFC 9562 version 4 layout.
    assert_eq!(
        run("const u = crypto.randomUUID();
             [u.length, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(u),
              crypto.randomUUID !== crypto.randomUUID() , crypto.randomUUID.length].join(' ')"),
        "36 true true 0"
    );
}

#[test]
fn random_uuid_is_there_in_a_secure_context_only() {
    // Chromium 153: an `http:` page that isn't local is no secure context,
    // and has no `randomUUID`; `https:` and localhost are. A page with no URL
    // is `about:blank`, which Secure Contexts §3.1 trusts.
    for (url, want) in [
        (None, "true function getRandomValues,randomUUID"),
        (
            Some("https://example.test/"),
            "true function getRandomValues,randomUUID",
        ),
        (
            Some("http://localhost:3000/"),
            "true function getRandomValues,randomUUID",
        ),
        (
            Some("http://probe.test/x"),
            "false undefined getRandomValues",
        ),
        (
            Some("file:///tmp/a.html"),
            "true function getRandomValues,randomUUID",
        ),
    ] {
        let got: String = runtime(url, 0)
            .eval(
                "[isSecureContext, typeof crypto.randomUUID,
                  Object.getOwnPropertyNames(Crypto.prototype).filter(k => k !== 'constructor').sort().join()].join(' ')",
            )
            .unwrap();
        assert_eq!(got, want, "{url:?}");
    }
    // No `subtle` anywhere.
    assert_eq!(
        run("typeof crypto.subtle + ' ' + ('subtle' in crypto)"),
        "undefined false"
    );
}

#[test]
fn every_new_global_gives_the_same_output_twice() {
    let body = "<script>
        log(crypto.getRandomValues(new Uint8Array(16)).join(), crypto.randomUUID(), Math.random());
        log(matchMedia('(max-width: 100px), print').media, CSS.escape('1a b'), CSS.supports('display:grid'));
        requestIdleCallback(d => log('idle', performance.now(), d.timeRemaining()));
        setTimeout(() => requestIdleCallback(d => log('late', performance.now(), d.timeRemaining()), { timeout: 3 }), 7);
        </script>";
    let a = page(body);
    assert_eq!(a, page(body));
    assert!(a.contains("idle 0 7"), "{a}");
}

#[test]
fn hostile_calls_are_bounded() {
    // A million lists, and a 10 MB query: past the cap it is `not all`.
    assert_eq!(
        long("let k = 0; for (let i = 0; i < 1e6; i++) if (matchMedia('(min-width: 1px)').matches) k++;
             const big = matchMedia('('.repeat(5e6) + ')'.repeat(5e6));
             const deep = matchMedia('('.repeat(1e4) + 'color' + ')'.repeat(1e4));
             [k, big.media, big.matches, deep.media, deep.matches].join(' ')"),
        "0 not all false not all false"
    );
    assert_eq!(
        long(
            "const s = 'a'.repeat(1 << 20);
             [CSS.supports(s), CSS.supports('--x', s)].join(' ')"
        ),
        "false false"
    );
    assert_eq!(
        long("String(CSS.supports('('.repeat(1e5) + 'color: red' + ')'.repeat(1e5)))"),
        "false"
    );
    assert_eq!(
        long("String(CSS.escape('\\0'.repeat(1e5)).length)"),
        "100000"
    );
    // getRandomValues on a huge buffer stops at the quota; on a million small
    // ones it is linear.
    assert_eq!(
        long("let n = 0; try { crypto.getRandomValues(new Uint8Array(1 << 22)) } catch (e) { n = e.code }
             const a = new Uint8Array(4); for (let i = 0; i < 1e6; i++) crypto.getRandomValues(a); String(n)"),
        "22"
    );
    // A valueOf that throws in the options is the page's own exception.
    assert_eq!(
        long("try { requestIdleCallback(() => {}, { timeout: { valueOf() { throw new RangeError('x') } } }); 'none' }
             catch (e) { e.name }"),
        "RangeError"
    );
}
