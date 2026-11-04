//! Events against the DOM Standard (§2 Events; sections cited per test) and
//! the page lifecycle against HTML §13.2.7. WPT dom/events files that hold
//! the same cases are named per test.

mod common;

use common::limits;
use surimono_core::SettleOptions;
use surimono_dom::{Document, DomError, DomRuntime};

const PAGE: &str = r#"<!doctype html><html><body><div id="a"><p id="b"><span id="c">x</span></p></div></body></html>"#;
const VARS: &str = "const a = document.getElementById('a'), b = document.getElementById('b'), c = document.getElementById('c');\nconst log = [];\n";

fn eval(script: &str) -> String {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    rt.eval::<String>(&format!("{VARS}{script}")).unwrap()
}

fn page(html: &str) -> (DomRuntime, surimono_dom::PageReport) {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(html).unwrap());
    let report = rt
        .run_page(&SettleOptions::default(), |src| {
            (src == "ok.js").then(|| "log.push('ext ' + document.readyState);".to_owned())
        })
        .unwrap();
    (rt, report)
}

#[test]
fn capture_target_bubble_order() {
    // §2.9; WPT dom/events/Event-dispatch-order.html, -bubbles-*.
    let r = eval(
        r#"const note = (n) => (e) => log.push(n + e.eventPhase);
        for (const [t, n] of [[document, 'D'], [a, 'A'], [b, 'B'], [c, 'C'], [window, 'W']]) {
          t.addEventListener('x', note(n + 'c'), true);
          t.addEventListener('x', note(n + 'b'));
        }
        c.dispatchEvent(new Event('x', { bubbles: true }));
        const seen = log.join(' ');
        log.length = 0;
        c.dispatchEvent(new Event('x'));
        seen + ' / ' + log.join(' ')"#,
    );
    assert_eq!(
        r,
        "Wc1 Dc1 Ac1 Bc1 Cc2 Cb2 Bb3 Ab3 Db3 Wb3 / Wc1 Dc1 Ac1 Bc1 Cc2 Cb2"
    );
}

#[test]
fn event_fields_during_and_after_dispatch() {
    // §2.2; WPT Event-constructors, Event-dispatch-target-*.
    let r = eval(
        r#"const e = new Event('x', { bubbles: true, cancelable: true });
        const d = [e.type, e.bubbles, e.cancelable, e.composed, e.target, e.currentTarget, e.eventPhase,
                   e.defaultPrevented, e.isTrusted, e.timeStamp >= 0, e instanceof Event, Event.AT_TARGET];
        a.addEventListener('x', function (ev) {
          log.push(this === a, ev === e, ev.target === c, ev.srcElement === c, ev.currentTarget === a, ev.eventPhase,
                   ev.composedPath().map((n) => n.nodeName || 'window').join('>'));
        });
        const r = c.dispatchEvent(e);
        [...d, r, ...log, e.target === c, e.currentTarget, e.eventPhase, e.composedPath().length].join()"#,
    );
    assert_eq!(
        r,
        "x,true,true,false,,,0,false,false,true,true,2,true,true,true,true,true,true,3,\
         SPAN>P>DIV>BODY>HTML>#document>window,true,,0,0"
    );
}

#[test]
fn stop_propagation() {
    // §2.2, §2.9; WPT Event-dispatch-propagation-stopped.html,
    // Event-stopImmediatePropagation.html, Event-cancelBubble.html.
    let r = eval(
        r#"const add = (t, n, f, o) => t.addEventListener('x', (e) => { log.push(n); if (f) f(e); }, o);
        add(a, 'a-cap', (e) => e.stopPropagation(), true);
        add(c, 'never');
        c.dispatchEvent(new Event('x', { bubbles: true }));
        const one = log.join(); log.length = 0;
        const lone = document.createElement('i');
        add(lone, 'i1', (e) => e.stopImmediatePropagation());
        add(lone, 'i2');
        lone.dispatchEvent(new Event('x'));
        const two = log.join(); log.length = 0;
        const e = new Event('y'); e.cancelBubble = true; log.push(e.cancelBubble);
        e.cancelBubble = false; log.push(e.cancelBubble);
        [one, two, ...log].join('|')"#,
    );
    assert_eq!(r, "a-cap|i1|true|true");
}

#[test]
fn prevent_default() {
    // §2.2; WPT Event-defaultPrevented.html, Event-returnValue.html,
    // AddEventListenerOptions-passive.html.
    let r = eval(
        r#"const cancel = (e) => e.preventDefault();
        a.addEventListener('x', cancel);
        const out = [a.dispatchEvent(new Event('x')), a.dispatchEvent(new Event('x', { cancelable: true }))];
        a.removeEventListener('x', cancel);
        a.addEventListener('y', cancel, { passive: true });
        const p = new Event('y', { cancelable: true });
        out.push(a.dispatchEvent(p), p.defaultPrevented);
        const v = new Event('z', { cancelable: true });
        out.push(v.returnValue); v.returnValue = false; out.push(v.returnValue, v.defaultPrevented);
        out.join()"#,
    );
    assert_eq!(r, "true,false,true,false,true,false,true");
}

#[test]
fn listener_list() {
    // §2.7; WPT EventTarget-addEventListener.any.js,
    // AddEventListenerOptions-once.html, EventListener-handleEvent.html,
    // Event-dispatch-handlers-changed.html.
    let r = eval(
        r#"const out = [];
        let n = 0;
        const f = () => n++;
        a.addEventListener('x', f); a.addEventListener('x', f); a.addEventListener('x', f, false);
        a.addEventListener('x', f, true);
        a.dispatchEvent(new Event('x')); out.push(n); n = 0;
        a.removeEventListener('x', f, true);
        a.dispatchEvent(new Event('x')); out.push(n); n = 0;
        a.removeEventListener('x', f);
        a.dispatchEvent(new Event('x')); out.push(n);
        a.addEventListener('o', f, { once: true });
        a.dispatchEvent(new Event('o')); a.dispatchEvent(new Event('o')); out.push(n); n = 0;
        const h = { handleEvent(e) { out.push(this === h, e.type); } };
        a.addEventListener('h', h); a.dispatchEvent(new Event('h'));
        a.addEventListener('n', null); a.addEventListener('n', undefined);
        // Removed during dispatch: not called. Added during: not called.
        const late = () => out.push('late'), gone = () => out.push('gone');
        a.addEventListener('c', () => { a.removeEventListener('c', gone); a.addEventListener('c', late); });
        a.addEventListener('c', gone);
        a.dispatchEvent(new Event('c'));
        const plain = new EventTarget();
        plain.addEventListener('p', () => out.push('plain'));
        plain.dispatchEvent(new Event('p'));
        for (const f of [() => a.dispatchEvent({}), () => a.dispatchEvent(), () => a.addEventListener('x'), () => new Event(),
                         () => Event('x'), () => new Event('x', 1)]) {
          try { f(); out.push('none'); } catch (e) { out.push(e.name); }
        }
        // WebIDL: a listener may be left out only with both arguments given.
        out.join()"#,
    );
    assert_eq!(
        r,
        "2,1,0,1,true,h,plain,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError"
    );
}

#[test]
fn dispatching_twice_is_an_error() {
    // §2.7 dispatchEvent; WPT EventTarget-dispatchEvent.html.
    let r = eval(
        r#"const e = new Event('x'), out = [];
        a.addEventListener('x', () => {
          try { a.dispatchEvent(e); out.push('none'); } catch (x) { out.push(x.name); }
          e.initEvent('other'); out.push(e.type);
        });
        a.dispatchEvent(e);
        out.push(a.dispatchEvent(e));
        const u = Object.create(Event.prototype);
        try { a.dispatchEvent(u); out.push('none'); } catch (x) { out.push(x.name); }
        out.join()"#,
    );
    assert_eq!(r, "InvalidStateError,x,InvalidStateError,x,true,TypeError");
}

#[test]
fn custom_events() {
    // §2.4; WPT CustomEvent.html.
    let r = eval(
        r#"const e = new CustomEvent('x', { detail: { n: 1 }, bubbles: true });
        const out = [];
        window.addEventListener('x', (ev) => out.push(ev.detail.n, ev instanceof CustomEvent, ev instanceof Event));
        c.dispatchEvent(e);
        out.push(new CustomEvent('y').detail);
        const i = new CustomEvent('y'); i.initCustomEvent('z', true, true, 5);
        out.push(i.type, i.bubbles, i.detail);
        class Mine extends Event { hi() { return 'hi ' + this.type; } }
        out.push(new Mine('m').hi());
        out.join()"#,
    );
    assert_eq!(r, "1,true,true,,z,true,5,hi m");
}

#[test]
fn window_is_the_global_event_target() {
    // HTML §7.2; WebIDL §3.7.6: an undefined `this` is the global object.
    let r = eval(
        r#"const out = [window === globalThis, self === window, window.window === window, window.document === document,
                     window instanceof EventTarget, window instanceof Window, devicePixelRatio, window.devicePixelRatio];
        addEventListener('g', function () { out.push(this === window); });
        window.dispatchEvent(new Event('g'));
        const add = EventTarget.prototype.addEventListener;
        add('h', () => out.push('undefined this'));
        dispatchEvent(new Event('h'));
        try { Window(); out.push('none'); } catch (e) { out.push(e.name); }
        out.push(typeof Window, Object.prototype.toString.call(window));
        out.join()"#,
    );
    assert_eq!(
        r,
        "true,true,true,true,true,true,1,1,true,undefined this,TypeError,function,[object Window]"
    );
}

#[test]
fn a_throwing_listener_is_reported_and_the_others_run() {
    // §2.9 "inner invoke": report the exception and go on; HTML §8.1.6.2:
    // an `error` event at the window. WPT
    // EventListener-handleEvent-cross-realm.html, event-global-*.
    let (mut rt, report) = page(
        r#"<!doctype html><script>
        globalThis.log = [];
        addEventListener('error', (e) => log.push('error: ' + e.message, e instanceof ErrorEvent, e.isTrusted));
        addEventListener('x', () => { throw new Error('boom'); });
        addEventListener('x', () => log.push('second'));
        dispatchEvent(new Event('x'));
        </script>"#,
    );
    assert_eq!(report.uncaught, 1);
    assert_eq!(report.errors, ["Error: boom"]);
    assert_eq!(
        rt.eval::<String>("log.join()").unwrap(),
        "error: Error: boom,true,true,second"
    );
    // A listener can cancel it, so nothing is counted.
    let (_, report) = page(
        r#"<!doctype html><script>
        addEventListener('error', (e) => e.preventDefault());
        addEventListener('x', () => { throw new Error('boom'); });
        dispatchEvent(new Event('x'));
        </script>"#,
    );
    assert_eq!(report.uncaught, 0);
}

const LIFECYCLE: &str = r#"<!doctype html><html><head><title>  a
  b </title>
<script>
globalThis.log = [];
const see = (w) => log.push(w + ' ' + document.readyState);
see('first');
document.addEventListener('readystatechange', (e) => see('change'));
document.addEventListener('DOMContentLoaded', (e) => { see('dcl'); log.push(e.target === document, e.bubbles, e.isTrusted); });
window.addEventListener('DOMContentLoaded', () => see('dcl at window'));
window.addEventListener('load', (e) => { see('load'); log.push(e.target === document, e.currentTarget === window); });
document.addEventListener('load', () => see('load at document: never'));
setTimeout(() => see('timer 0'), 0);
Promise.resolve().then(() => see('microtask'));
</script>
<script defer src="ok.js"></script>
<script src="ok.js" id="e" onerror="1"></script>
<script>
document.getElementById('e').addEventListener('load', () => log.push('script load'));
</script>
<script src="missing.js" id="m"></script>
<script>const shared = 'across scripts';</script>
<script type="text/plain">log.push('never');</script>
<script type="module">log.push('never');</script>
<script>see('last ' + shared + ' ' + document.title + ' ' + document.querySelectorAll('p').length);
document.getElementById('m').addEventListener('error', () => log.push('never: already fired'));
</script>
</head><body><p>after the scripts</p></body></html>"#;

#[test]
fn the_page_lifecycle_runs_in_a_defined_order() {
    let (mut rt, report) = page(LIFECYCLE);
    assert_eq!(report.uncaught, 0, "{:?}", report.errors);
    assert_eq!(report.missing_scripts, ["missing.js"]);
    let log: String = rt.eval("log.join('\\n')").unwrap();
    assert_eq!(
        log.lines().collect::<Vec<_>>(),
        [
            "first loading",
            "microtask loading",
            "ext loading",
            "last across scripts a b 1 loading",
            "change interactive",
            "ext interactive",
            "dcl interactive",
            "true",
            "true",
            "true",
            "dcl at window interactive",
            "change complete",
            "load complete",
            "true",
            "true",
            "timer 0 complete",
        ]
    );
}

#[test]
fn title_and_ready_state() {
    // HTML §3.1.3: title; a document no page ran on is complete.
    let r = eval(
        r#"const out = [document.title, document.readyState];
        document.title = 'made'; out.push(document.title, document.querySelector('head title').textContent);
        document.title = ' x   y '; out.push(document.title, document.head.firstChild.textContent);
        out.join('|')"#,
    );
    assert_eq!(r, "|complete|made|made|x y| x   y ");
}

#[test]
fn event_loop_through_the_runtime() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let _: () = rt
        .eval("setTimeout(() => { document.getElementById('c').textContent = 'later'; }, 500);")
        .unwrap();
    let report = rt.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(report.virtual_now_ms, 500);
    let html = rt.with_document(Document::to_html).unwrap();
    assert!(html.contains(">later<"), "{html}");
}

#[test]
fn limits_in_a_page_end_it_with_a_typed_error_and_the_runtime_recovers() {
    let mut rt =
        DomRuntime::new(limits().with_time(std::time::Duration::from_millis(300))).unwrap();
    rt.load(Document::parse("<!doctype html><script>for (;;) {}</script>").unwrap());
    let r = rt.run_page(&SettleOptions::default(), |_| None);
    assert_eq!(
        r.unwrap_err(),
        DomError::Sandbox(surimono_core::SandboxError::TimeLimit)
    );
    // The bindings come back after the sandbox rebuilt itself.
    assert!(rt.eval::<bool>("document !== null").unwrap());
    // A DOM limit in a timer callback is as fatal as in a script.
    rt.load(
        Document::parse_with_limits(
            "<!doctype html><script>setTimeout(() => { try { for (;;) document.createElement('b'); } catch (e) {} }, 0);</script>",
            surimono_dom::DomLimits {
                max_nodes: 50,
                ..surimono_dom::DomLimits::default()
            },
        )
        .unwrap(),
    );
    let r = rt.run_page(&SettleOptions::default(), |_| None);
    assert_eq!(r.unwrap_err(), DomError::NodeLimit { limit: 50 });
}
