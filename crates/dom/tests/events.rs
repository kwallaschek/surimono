//! Events against the DOM Standard (§2 Events; sections cited per test) and
//! the page lifecycle against HTML §13.2.7. WPT dom/events files that hold
//! the same cases are named per test.

mod common;

use std::time::Duration;

use common::limits;
use surimono_core::{SandboxError, SettleOptions};
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

/// Runs `html` as a page; the result stays with the caller, for typed ends.
fn run(html: &str) -> Result<surimono_dom::PageReport, DomError> {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(html).unwrap());
    rt.run_page(&SettleOptions::default(), |_| None)
}

fn log_of(rt: &mut DomRuntime) -> String {
    rt.eval::<String>("globalThis.log.join()").unwrap()
}

/// HTML §13.2.7 has no way to cancel "the end": the idiom that clears every
/// timer a page may have set must leave it alone.
#[test]
fn a_page_cant_cancel_its_own_lifecycle() {
    for clear in ["clearTimeout", "clearInterval"] {
        let (mut rt, report) = page(&format!(
            r#"<!doctype html><script>
            globalThis.log = [];
            let id = setTimeout(() => log.push('cleared timer'), 0);
            while (id--) {clear}(id + 1);
            {clear}('1'); {clear}(2.5);
            addEventListener('DOMContentLoaded', () => log.push('dcl'));
            addEventListener('load', () => log.push('load ' + document.readyState));
            setTimeout(() => log.push('own timer'), 0);
            </script><script>log.push('second script');</script>"#
        ));
        assert_eq!(report.uncaught, 0, "{:?}", report.errors);
        assert_eq!(
            log_of(&mut rt),
            "second script,dcl,load complete,own timer",
            "{clear}"
        );
    }
}

/// HTML §8.7: a timer's nesting level starts at 0 in a script, and only
/// timers past level 5 are clamped to 4 ms (WPT
/// html/webappapis/timers/ `nested-*`), so the chain's seventh step is the
/// first to move the clock.
#[test]
fn a_script_timer_starts_at_nesting_level_zero() {
    let (mut rt, _) = page(
        r#"<!doctype html><script>
        globalThis.log = [];
        const t0 = performance.now();
        let n = 0;
        function tick() { log.push(performance.now() - t0); if (++n < 8) setTimeout(tick, 0); }
        setTimeout(tick, 0);
        </script>"#,
    );
    assert_eq!(log_of(&mut rt), "0,0,0,0,0,0,4,8");
}

/// HTML §8.1.6.2: in error reporting mode, no second event.
#[test]
fn an_error_listener_that_throws_doesnt_recurse() {
    let (mut rt, report) = page(
        r#"<!doctype html><script>
        globalThis.log = [];
        addEventListener('error', (e) => { log.push(e.message); nosuchfn(); });
        addEventListener('error', () => { log.push('second'); throw 1; });
        throw new Error('first');
        </script><script>log.push('next script');</script>"#,
    );
    assert_eq!(report.uncaught, 3, "{:?}", report.errors);
    assert_eq!(
        report.errors,
        [
            "ReferenceError: nosuchfn is not defined",
            "1",
            "Error: first"
        ]
    );
    assert_eq!(log_of(&mut rt), "Error: first,second,next script");
}

const OVERFLOW: DomError = DomError::Sandbox(SandboxError::StackOverflow);

/// The same limit ends the call the same way on every path, and an `error`
/// listener neither sees nor cancels it.
#[test]
fn an_uncaught_stack_overflow_ends_the_page_on_every_path() {
    const HIDE: &str = "<!doctype html><script>globalThis.log = [];
        addEventListener('error', (e) => { log.push('saw it'); e.preventDefault(); });
        window.onerror = () => true;
        function f() { f(); }
        </script>";
    for (name, start) in [
        ("script", "f();"),
        (
            "listener",
            "addEventListener('x', f); dispatchEvent(new Event('x'));",
        ),
        ("load", "addEventListener('load', f);"),
        ("timer", "setTimeout(f, 0);"),
        ("string timer", "setTimeout('f()', 0);"),
        ("frame", "requestAnimationFrame(f);"),
        ("microtask", "queueMicrotask(f);"),
        ("handler", "document.onreadystatechange = f;"),
    ] {
        let html = format!("{HIDE}<script>{start}</script>");
        let mut rt = DomRuntime::new(limits()).unwrap();
        rt.load(Document::parse(&html).unwrap());
        assert_eq!(
            rt.run_page(&SettleOptions::default(), |_| None)
                .unwrap_err(),
            OVERFLOW,
            "{name}"
        );
        // The script's state went, as after a limit in a timer, and the
        // runtime still works.
        assert_eq!(rt.eval::<String>("typeof log").unwrap(), "undefined");
        assert_eq!(rt.eval::<i32>("1 + 1").unwrap(), 2);
    }
}

/// The script doesn't run on after the limit: the spin in the next script
/// gets no chance to end in `TimeLimit`.
#[test]
fn nothing_runs_after_an_uncaught_stack_overflow() {
    for start in ["f();", "queueMicrotask(f);"] {
        let html = format!(
            "<!doctype html><script>function f() {{ f(); }} {start}</script>
             <script>for (;;) document.title = 'x';</script>"
        );
        let mut rt = DomRuntime::new(limits()).unwrap();
        rt.load(Document::parse(&html).unwrap());
        let r = rt.run_page(&SettleOptions::default(), |_| None);
        assert_eq!(r.unwrap_err(), OVERFLOW, "{start}");
        let html = rt.with_document(Document::to_html).unwrap();
        assert!(!html.contains("<title>"), "{html}");
    }
}

/// `eval`, outside a page: a listener's overflow is as fatal.
#[test]
fn a_listener_overflow_fails_eval_too() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    let r = rt.eval::<()>(
        "function f() { f(); } addEventListener('x', f); dispatchEvent(new Event('x'));",
    );
    assert_eq!(r.unwrap_err(), OVERFLOW);
    assert_eq!(rt.eval::<String>("typeof f").unwrap(), "undefined");
}

/// A script may catch its own overflow, as in a browser: nothing was uncaught.
#[test]
fn a_caught_stack_overflow_is_a_script_matter() {
    let (mut rt, report) = page(
        r#"<!doctype html><script>
        globalThis.log = [];
        function f() { f(); }
        try { f(); } catch (e) { log.push(e.name); }
        </script><script>log.push('next');</script>"#,
    );
    assert_eq!(report.uncaught, 0);
    assert_eq!(log_of(&mut rt), "RangeError,next");
}

/// Only the engine can say a value is a limit error: one a script spells is
/// an ordinary exception, on every path.
#[test]
fn limits_spelled_by_a_script_are_ordinary_exceptions() {
    let fake = "{ name: 'InternalError', message: 'out of memory' }";
    for start in [
        format!("throw {fake};"),
        format!("setTimeout(() => {{ throw {fake}; }}, 0);"),
        format!("queueMicrotask(() => {{ throw {fake}; }});"),
        "throw new RangeError('Maximum call stack size exceeded');".to_owned(),
    ] {
        let html = format!("<!doctype html><script>{start}</script>");
        assert_eq!(run(&html).unwrap().uncaught, 1, "{start}");
    }
    let html = format!("<!doctype html><script>Promise.reject({fake});</script>");
    assert_eq!(run(&html).unwrap().unhandled_rejections, 1);
}

/// And renaming a real one, or making it overflow only once it is read,
/// hides nothing.
#[test]
fn a_disguised_limit_error_still_ends_the_page() {
    let f = "function f() { f(); }";
    for start in [
        "try { f(); } catch (e) { e.name = 'Error'; e.message = 'fine'; throw e; }",
        "setTimeout(() => { try { f(); } catch (e) { e.name = 'x'; throw e; } }, 0);",
        "throw { toString() { f(); } };",
    ] {
        let html = format!(
            "<!doctype html><script>{f} addEventListener('error', (e) => e.preventDefault());
             {start}</script>"
        );
        assert_eq!(run(&html).unwrap_err(), OVERFLOW, "{start}");
    }
}

/// A limit that rejects a promise ends the page as a thrown one does: the
/// paths of `an_uncaught_stack_overflow_ends_the_page_on_every_path`, with
/// the function async, or the overflow in a reaction.
#[test]
fn a_limit_in_a_promise_ends_the_page_on_every_path() {
    const HIDE: &str = "<!doctype html><script>globalThis.log = [];
        addEventListener('error', (e) => { log.push('saw it'); e.preventDefault(); });
        window.onerror = () => true;
        function f() { return f() + 1; }
        function grow() { const a = []; for (;;) a.push(new Array(1000).fill(a.length)); }
        async function af() { f(); }
        async function ag() { grow(); }
        </script>";
    for (name, start, error) in [
        (
            "async function",
            "(async function () { f(); })();",
            OVERFLOW,
        ),
        (
            "after an await",
            "(async () => { await null; f(); })();",
            OVERFLOW,
        ),
        ("reaction", "Promise.resolve().then(f);", OVERFLOW),
        (
            "rejection reaction",
            "Promise.reject(1).catch(f);",
            OVERFLOW,
        ),
        ("executor", "new Promise(f);", OVERFLOW),
        (
            "async listener",
            "addEventListener('x', af); dispatchEvent(new Event('x'));",
            OVERFLOW,
        ),
        ("async load", "addEventListener('load', af);", OVERFLOW),
        (
            "async handler",
            "document.onreadystatechange = af;",
            OVERFLOW,
        ),
        ("async timer", "setTimeout(af, 0);", OVERFLOW),
        ("async frame", "requestAnimationFrame(af);", OVERFLOW),
        ("async microtask", "queueMicrotask(af);", OVERFLOW),
        (
            "async error listener",
            "addEventListener('error', af); throw 1;",
            OVERFLOW,
        ),
        (
            "memory",
            "ag();",
            DomError::Sandbox(SandboxError::MemoryLimit),
        ),
        (
            "memory in a reaction",
            "Promise.resolve().then(grow);",
            DomError::Sandbox(SandboxError::MemoryLimit),
        ),
    ] {
        let html = format!("{HIDE}<script>{start}</script>");
        let mut rt = DomRuntime::new(limits()).unwrap();
        rt.load(Document::parse(&html).unwrap());
        let r = rt.run_page(&SettleOptions::default(), |_| None);
        assert_eq!(r.unwrap_err(), error, "{name}");
        // The script's state went, and the runtime still works.
        assert_eq!(rt.eval::<String>("typeof log").unwrap(), "undefined");
        assert_eq!(rt.eval::<i32>("1 + 1").unwrap(), 2);
    }
}

/// The page stops there: the spin in the next script never starts.
#[test]
fn nothing_runs_after_a_limit_in_a_promise() {
    let html = "<!doctype html><script>function f() { f(); } (async () => f())();</script>
        <script>for (;;) document.title = 'x';</script>";
    assert_eq!(run(html).unwrap_err(), OVERFLOW);
}

/// `eval`, outside a page: its script is a task, with the checkpoint at the
/// end, so the call that ran the promise is the one that fails.
#[test]
fn a_limit_in_a_promise_fails_eval_too() {
    const F: &str = "function f() { f(); } async function af() { f(); }\n";
    for start in [
        "af();",
        "Promise.resolve().then(f);",
        "addEventListener('x', af); dispatchEvent(new Event('x'));",
        "af(); throw 1;",
    ] {
        let mut rt = DomRuntime::new(limits()).unwrap();
        rt.load(Document::parse(PAGE).unwrap());
        let r = rt.eval::<()>(&format!("{F}{start}"));
        assert_eq!(r.unwrap_err(), OVERFLOW, "{start}");
        assert_eq!(rt.eval::<String>("typeof f").unwrap(), "undefined");
    }
}

/// Handling the promise is catching the error, in time or not at all: at
/// the end of the task a handler still counts, a task later it doesn't.
#[test]
fn a_handled_limit_in_a_promise_is_a_script_matter() {
    let (mut rt, report) = page(
        r#"<!doctype html><script>
        globalThis.log = [];
        function f() { f(); }
        async function af() { f(); }
        af().catch((e) => log.push(e.name));
        const later = af();
        queueMicrotask(() => later.catch((e) => log.push('late ' + e.name)));
        </script><script>log.push('next');</script>"#,
    );
    assert_eq!((report.uncaught, report.unhandled_rejections), (0, 0));
    assert_eq!(log_of(&mut rt), "RangeError,late RangeError,next");

    let html = "<!doctype html><script>function f() { f(); } globalThis.p = (async () => f())();
        </script><script>p.catch(() => {});</script>";
    assert_eq!(run(html).unwrap_err(), OVERFLOW);
}

/// HTML "unhandled promise rejections": what no script handled reaches the
/// report, an `error` listener doesn't see it, and the page goes on.
#[test]
fn rejections_nothing_handled_reach_the_report() {
    let (mut rt, report) = page(
        r#"<!doctype html><script>
        globalThis.log = [];
        addEventListener('error', () => log.push('error event'));
        (async () => { throw new TypeError('in async'); })();
        Promise.reject('plain');
        Promise.reject(new Error('handled')).catch(() => {});
        const p = Promise.reject(new Error('handled later'));
        queueMicrotask(() => p.catch(() => {}));
        addEventListener('x', async () => { throw new Error('in listener'); });
        dispatchEvent(new Event('x'));
        </script><script>log.push('next');</script>"#,
    );
    assert_eq!(report.uncaught, 0);
    assert_eq!(report.unhandled_rejections, 3);
    assert_eq!(
        report.rejections,
        ["TypeError: in async", "plain", "Error: in listener"]
    );
    assert_eq!(report.settle.unhandled_rejections, 3);
    assert_eq!(log_of(&mut rt), "next");
}

#[test]
fn reported_rejections_are_capped() {
    let (_, report) =
        page("<!doctype html><script>for (let i = 0; i < 20; i++) Promise.reject(i);</script>");
    assert_eq!(report.unhandled_rejections, 20);
    assert_eq!(report.rejections.len(), 8);
}

/// A rejection in an `eval` waits for the next report, once, in order, and
/// ends no timer: `eval`'s checkpoint runs no task.
#[test]
fn eval_keeps_its_rejections_for_the_next_report() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(PAGE).unwrap());
    rt.eval::<()>(
        "globalThis.log = []; Promise.reject('one');
         setTimeout(() => { log.push('timer'); Promise.reject('two'); }, 0);",
    )
    .unwrap();
    assert_eq!(rt.eval::<u32>("log.length").unwrap(), 0);
    let report = rt.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(report.unhandled_rejections, 2);
    let reasons: Vec<String> = report.rejections.iter().map(|e| e.to_string()).collect();
    assert!(
        reasons[0].contains("one") && reasons[1].contains("two"),
        "{reasons:?}"
    );
    assert_eq!(log_of(&mut rt), "timer");
    let report = rt.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(report.unhandled_rejections, 0);
}

/// `prepare the script element` picks what runs (HTML §4.12.1.1); the MIME
/// list is MIME Sniffing §4.6. WPT html/semantics/scripting-1/the-script-element/
/// script-type-and-language-js.html, script-languages-*.
#[test]
fn which_scripts_run() {
    for (attrs, runs) in [
        ("", true),
        ("type=''", true),
        ("language=''", true),
        ("type='' language='vbscript'", true),
        ("language='JavaScript'", true),
        ("language='javascript1.5'", true),
        ("language='livescript'", true),
        ("language='vbscript'", false),
        ("language=' javascript'", false),
        ("type='text/javascript' language='vbscript'", true),
        ("type='application/x-ecmascript'", true),
        ("type='TEXT/JavaScript1.5'", true),
        ("type='text/livescript'", true),
        ("type='text/javascript1.0'", true),
        ("type='text/javascript1.6'", false),
        ("type='\t text/jscript\n'", true),
        ("type='\u{a0}text/javascript'", false),
        ("type=' '", false),
        ("type='text/javascript;charset=utf-8'", false),
        ("type='javascript'", false),
        ("type='module'", false),
        ("type='text/plain'", false),
    ] {
        let html = format!(
            "<!doctype html><script>globalThis.log = [];</script>
             <script {attrs}>log.push('ran');</script><script>log.push('end');</script>"
        );
        let (mut rt, report) = page(&html);
        assert_eq!(report.uncaught, 0, "{attrs} {:?}", report.errors);
        let expected = if runs { "ran,end" } else { "end" };
        assert_eq!(log_of(&mut rt), expected, "{attrs}");
    }
}

/// HTML §4.12.3: template contents are inert. §13.2.6.4.4, §13.2.6.4.7: with
/// scripting on `<noscript>` is raw text, so no script element exists there.
#[test]
fn nothing_runs_inside_template_or_noscript() {
    let (mut rt, report) = page(
        r#"<!doctype html><html><head><script>globalThis.log = [];</script>
        <noscript><script>log.push('noscript in head');</script></noscript>
        <template><script>log.push('template in head');</script></template></head>
        <body><noscript><div><script>log.push('noscript in body');</script></div></noscript>
        <template><div><script>log.push('template in body');</script></div></template>
        <script>log.push('end ' + document.querySelectorAll('noscript script').length
          + ' ' + document.querySelector('body noscript').textContent.includes('<div>'));</script></body></html>"#,
    );
    assert_eq!(report.uncaught, 0, "{:?}", report.errors);
    assert_eq!(log_of(&mut rt), "end 0 true");
}

/// §4.12.1.1: an empty script does nothing at all; `async` overrides `defer`.
#[test]
fn empty_and_async_scripts() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse(
            r#"<!doctype html><script>globalThis.log = [];
            addEventListener('DOMContentLoaded', () => log.push('dcl'));
            </script><script id=e></script>
            <script>document.getElementById('e').addEventListener('load', () => log.push('never'));</script>
            <script src=d.js defer></script><script src=a.js defer async></script>
            <script>log.push('inline');</script>"#,
        )
        .unwrap(),
    );
    rt.run_page(&SettleOptions::default(), |src| {
        Some(format!("log.push('{src}');"))
    })
    .unwrap();
    assert_eq!(log_of(&mut rt), "a.js,inline,d.js,dcl");
}

/// HTML §8.7, §8.8, §8.10.1 "report the exception": callbacks of the event
/// loop go through the same `error` event as scripts.
#[test]
fn exceptions_in_callbacks_reach_error_listeners_and_the_report() {
    let (mut rt, report) = page(
        r#"<!doctype html><script>
        globalThis.log = [];
        addEventListener('error', (e) => { log.push(e.message); if (e.message.endsWith('cancelled')) e.preventDefault(); });
        setTimeout(() => { throw new Error('timer'); }, 0);
        setTimeout("throw new Error('string')", 0);
        setTimeout('syntax error', 0);
        requestAnimationFrame(() => { throw new Error('frame'); });
        queueMicrotask(() => { throw new Error('microtask'); });
        queueMicrotask(() => { throw new Error('cancelled'); });
        </script>"#,
    );
    assert_eq!(report.uncaught, 5, "{:?}", report.errors);
    assert_eq!(report.settle.uncaught, 0);
    assert_eq!(
        report.errors,
        [
            "Error: microtask",
            "Error: timer",
            "Error: string",
            "SyntaxError: expecting ';'",
            "Error: frame",
        ]
    );
    assert_eq!(
        log_of(&mut rt),
        "Error: microtask,Error: cancelled,Error: timer,Error: string,SyntaxError: expecting ';',Error: frame"
    );
}

/// What timers took and how they behave doesn't change with the wrappers.
#[test]
fn timers_keep_their_shape() {
    let (mut rt, _) = page(
        r#"<!doctype html><script>
        globalThis.log = [];
        for (const f of [setTimeout, setInterval, clearTimeout, clearInterval, requestAnimationFrame, queueMicrotask]) {
          const d = Object.getOwnPropertyDescriptor(window, f.name);
          log.push(f.name, f.length, d.writable, d.enumerable, d.configurable, 'prototype' in f);
        }
        for (const f of [() => setTimeout(), () => setInterval(), () => queueMicrotask(1), () => requestAnimationFrame(1)]) {
          try { f(); log.push('none'); } catch (e) { log.push(e.name); }
        }
        const order = [];
        setTimeout({ toString() { order.push('handler'); return ''; } }, { valueOf() { order.push('timeout'); return 0; } });
        log.push(order.join('+'));
        setTimeout(function (a, b) { log.push(this === window, a, b); }, 0, 'x', 'y');
        let n = 0;
        const i = setInterval(() => { if (++n === 3) { clearInterval(i); log.push('interval ' + n); } }, 1);
        clearTimeout(); clearTimeout(undefined); clearInterval(null);
        </script>"#,
    );
    assert_eq!(
        log_of(&mut rt),
        "setTimeout,1,true,true,true,false,\
         setInterval,1,true,true,true,false,\
         clearTimeout,0,true,true,true,false,\
         clearInterval,0,true,true,true,false,\
         requestAnimationFrame,1,true,true,true,false,\
         queueMicrotask,1,true,true,true,false,\
         TypeError,TypeError,TypeError,TypeError,handler+timeout,true,x,y,interval 3"
    );
}

/// A long exception message doesn't reach Rust whole.
#[test]
fn reported_messages_are_capped() {
    let (_, report) = page(
        r#"<!doctype html><script>throw new Error('é'.repeat(50000));</script>
        <script>setTimeout(() => { throw 'é'.repeat(50000); }, 0);</script>"#,
    );
    assert_eq!(report.uncaught, 2);
    for e in &report.errors {
        assert!(e.len() <= 4 * 1024 + 3, "{}", e.len());
        assert!(e.ends_with('…'));
    }
}

/// HTML §8.1.7.1 on window, document and elements: the IDL attributes.
#[test]
fn event_handler_idl_attributes() {
    let r = eval(
        r#"const out = [];
        // WPT html/webappapis/scripting/events/event-handler-spec-example.window.js
        for (const v of [{}, function () {}, new Number(42), new String()]) {
          let i = 0;
          const n = () => { throw new Error('first handler'); };
          const b = document.createElement('button');
          b.onclick = v;
          out.push(b.onclick === v);
          b.addEventListener('click', () => out.push(++i === 2));
          b.setAttribute('onclick', 'nosuch();');
          b.addEventListener('click', () => out.push(++i === 3));
          b.onclick = () => { out.push(++i === 1); };
          b.addEventListener('click', () => out.push(++i === 4));
          b.dispatchEvent(new Event('click'));
          out.push(i, b.getAttribute('onclick'));
        }
        const j = out.join(); out.length = 0;
        for (const v of [42, null, undefined, '']) { a.onclick = v; out.push(a.onclick); }
        // Null deactivates: the next set goes last.
        a.onfoo = 1;
        a.onclick = () => log.push('h1');
        a.addEventListener('click', () => log.push('l'));
        a.onclick = null;
        a.onclick = function (e) { log.push('h2', this === a, e.type); };
        a.dispatchEvent(new Event('click'));
        // WPT event-handler-processing-algorithm.html.
        a.onmouseover = () => false;
        const e = new Event('mouseover', { cancelable: true });
        a.dispatchEvent(e); out.push(e.defaultPrevented);
        a.onmouseover = () => true;
        const t = new Event('mouseover', { cancelable: true });
        a.dispatchEvent(t); out.push(t.defaultPrevented);
        // Something that isn't callable is kept and does nothing.
        a.onmouseover = {};
        a.dispatchEvent(new Event('mouseover'));
        document.onreadystatechange = () => log.push('doc');
        document.dispatchEvent(new Event('readystatechange'));
        out.push('onclick' in a, 'onclick' in document, 'onclick' in window, 'onhashchange' in window,
                 'onhashchange' in a, 'onreadystatechange' in a, 'onreadystatechange' in document,
                 'onclick' in document.createTextNode('x'));
        try { Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'onclick').get.call({}); out.push('none'); }
        catch (x) { out.push(x.name); }
        [j, out.join(), log.join()].join('|')"#,
    );
    let once = "true,true,true,true,true,4,nosuch();";
    assert_eq!(
        r,
        format!(
            "{once},{once},{once},{once}|,,,,true,false,true,true,true,true,false,false,true,false,TypeError|l,h2,true,click,doc"
        )
    );
}

/// HTML §8.1.7.1, special error event handling, and the window's handlers.
#[test]
fn window_handlers_from_a_page() {
    let (mut rt, report) = page(
        r#"<!doctype html><script>
        globalThis.log = [];
        window.onload = (e) => log.push('onload ' + (e.target === document));
        document.onreadystatechange = () => log.push(document.readyState);
        window.onerror = (m, f, l, c, e) => { log.push(m, e instanceof Error); return m === 'Error: handled'; };
        </script><script>throw new Error('handled');</script><script>throw new Error('counted');</script>"#,
    );
    assert_eq!(report.uncaught, 1, "{:?}", report.errors);
    assert_eq!(report.errors, ["Error: counted"]);
    assert_eq!(
        log_of(&mut rt),
        "Error: handled,true,Error: counted,true,interactive,complete,onload true"
    );
}

/// HTML §8.1.7.1, event handler content attributes: parsed, set later and
/// removed; `this`, `event`, the scope chain and `return false`.
#[test]
fn event_handler_content_attributes() {
    let (mut rt, report) = page(
        r#"<!doctype html><html><body onload="log.push('body onload', event.type, this === window)">
        <div id=d title=t onclick="log.push('attr', this.id, title, typeof createElement, event.type); return false" onbogus="log.push('never')"></div>
        <script>
        globalThis.log = [];
        const d = document.getElementById('d');
        d.addEventListener('click', () => log.push('listener'));
        const e = new Event('click', { cancelable: true });
        d.dispatchEvent(e);
        log.push(e.defaultPrevented, typeof d.onclick, d.onclick.name, d.onclick === d.onclick, d.onbogus === undefined);
        d.dispatchEvent(new Event('bogus'));
        d.setAttribute('onclick', "log.push('changed')");
        d.dispatchEvent(new Event('click'));
        d.onclick = () => log.push('idl');
        d.dispatchEvent(new Event('click'));
        d.setAttribute('onclick', "log.push('again')");
        d.dispatchEvent(new Event('click'));
        d.removeAttribute('onclick');
        log.push(d.onclick);
        d.dispatchEvent(new Event('click'));
        const b = document.createElement('div');
        b.innerHTML = '<i onclick="log.push(1 + 1)"></i>';
        b.firstChild.dispatchEvent(new Event('click'));
        </script>"#,
    );
    assert_eq!(report.uncaught, 0, "{:?}", report.errors);
    assert_eq!(
        log_of(&mut rt),
        "attr,d,t,function,click,listener,true,function,onclick,true,true,\
         changed,listener,idl,listener,again,listener,,listener,2,body onload,load,true"
    );
}

/// WPT html/webappapis/scripting/events/invalid-uncompiled-raw-handler-compiled-once.window.js.
#[test]
fn a_bad_handler_attribute_is_compiled_once() {
    let (mut rt, report) = page(
        r#"<!doctype html><body><script>
        globalThis.log = [];
        window.onerror = () => { log.push('error'); return true; };
        const e = document.body;
        e.setAttribute('onclick', 'window.open(');
        log.push(e.onclick, e.onclick);
        e.setAttribute('onmouseover', '}; log.push("at compile"); function x() {');
        log.push(e.onmouseover);
        </script>"#,
    );
    assert_eq!(report.uncaught, 0, "{:?}", report.errors);
    assert_eq!(log_of(&mut rt), "error,,,error,");
}

/// HTML §8.1.7.1: on a body element the Window-reflecting handlers are the
/// window's. WPT event-handler-attributes-body-window.html.
#[test]
fn body_handlers_are_the_windows() {
    let r = eval(
        r#"const out = [];
        const f = () => {};
        document.body.onload = f;
        out.push(window.onload === f, document.createElement('body').onload === f);
        window.onerror = f;
        out.push(document.body.onerror === f);
        document.body.onclick = f;
        out.push(window.onclick === null);
        a.onload = f;
        window.onload = null;
        out.push(a.onload === f, document.body.onload);
        out.join()"#,
    );
    assert_eq!(r, "true,true,true,true,true,");
}

/// HTML §8.1.3.5: the element of the running script; null outside one.
#[test]
fn current_script() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse(
            r#"<!doctype html><script id=a>globalThis.log = [];
            const see = () => { const s = document.currentScript; log.push(s === null ? 'null' : s.id); };
            see();
            queueMicrotask(see);
            setTimeout(see, 0);
            addEventListener('load', see);
            addEventListener('DOMContentLoaded', see);
            </script><script id=b src=e.js></script><script id=c defer src=e.js></script>"#,
        )
        .unwrap(),
    );
    rt.run_page(&SettleOptions::default(), |_| Some("see();".to_owned()))
        .unwrap();
    assert_eq!(log_of(&mut rt), "a,a,b,c,null,null,null");
    // No page is running: `eval` has no current script.
    assert!(rt.eval::<bool>("document.currentScript === null").unwrap());
}

/// HTML §8.1.4.4 "run a classic script" and "clean up after running script":
/// the microtask checkpoint comes before `currentScript` is restored and
/// before the element's `load`, however long the chain.
#[test]
fn a_scripts_microtasks_are_part_of_its_run() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(
        Document::parse(
            r#"<!doctype html><script id=a>globalThis.log = [];
            globalThis.see = (at) => () => {
                const s = document.currentScript;
                log.push(at + ':' + (s === null ? 'null' : s.id));
            };
            (async () => {
                await null;
                see('await')();
                await null;
                Promise.resolve().then(see('then')).then(see('chained'));
            })();
            </script><script id=b src=e.js onload="see('load')()"></script>
            <script id=c>addEventListener('error', see('error')); throw 1;</script>"#,
        )
        .unwrap(),
    );
    rt.run_page(&SettleOptions::default(), |_| {
        Some("queueMicrotask(see('external'));".to_owned())
    })
    .unwrap();
    assert_eq!(
        log_of(&mut rt),
        "await:a,then:a,chained:a,external:b,load:null,error:c"
    );
}

/// DOM §2.5, §2.7.3; WebIDL §3.2.17, §3.7.6; HTML §3.1.3.
#[test]
fn small_spec_points() {
    let r = eval(
        r#"const out = [];
        // "initialize" keeps composed and timeStamp.
        const e = new Event('x', { composed: true });
        const t = e.timeStamp;
        e.initEvent('y', true, true);
        out.push(e.composed, e.timeStamp === t, e.type, e.bubbles);
        // A function is an object, so it is the options dictionary.
        const opts = () => {}; opts.capture = true; opts.once = true;
        let n = 0;
        const f = () => n++;
        a.addEventListener('c', f, opts);
        a.dispatchEvent(new Event('c')); a.dispatchEvent(new Event('c'));
        out.push(n);
        a.addEventListener('d', f, opts);
        a.removeEventListener('d', f, opts);
        a.dispatchEvent(new Event('d'));
        out.push(n);
        // A null `this` is the global object.
        const add = EventTarget.prototype.addEventListener;
        add.call(null, 'g', () => out.push('window'));
        dispatchEvent(new Event('g'));
        // Child text content: `title` isn't a descendant walk.
        const title = document.createElement('title');
        title.append('a', document.createElement('b'), 'c');
        title.lastChild.before(document.createElement('b'));
        title.children[0].textContent = 'not this';
        document.head.append(title);
        out.push(document.title);
        out.join()"#,
    );
    assert_eq!(r, "true,true,y,true,1,1,window,ac");
}

#[test]
fn event_constructors_follow_their_dictionaries() {
    // DOM §2.5; UI Events; Pointer Events; WPT
    // dom/events/Event-subclasses-constructors.html.
    let r = eval(
        r#"const f = new FocusEvent('f', { relatedTarget: document, view: window, detail: '7' });
        const m = new MouseEvent('m', { screenX: 1.5, clientY: 2, button: 300, buttons: 65537,
          ctrlKey: 1, modifierCapsLock: true, relatedTarget: a });
        const w = new WheelEvent('w', { deltaY: 3.5, deltaMode: 1 });
        const k = new KeyboardEvent('k', { key: 'a', code: 'KeyA', location: 2, repeat: true });
        const p = new PointerEvent('p', { pointerId: 4, pointerType: 'pen', width: 3, clientX: 9 });
        const h = new HashChangeEvent('h', { oldURL: 'a', newURL: 'b' });
        const cm = new CompositionEvent('c', { data: 'd' });
        const i = new InputEvent('i', { data: 'x', inputType: 'insertText', isComposing: true });
        const s = new StorageEvent('s', { key: 'k', url: 'u' });
        [f.view === window, f.detail, f.relatedTarget === document, f instanceof UIEvent,
         m.screenX, m.clientY, m.button, m.buttons, m.ctrlKey, m.getModifierState('Control'),
         m.getModifierState('CapsLock'), m.getModifierState('Shift'), m.relatedTarget === a,
         m.pageX === m.clientX, m.y,
         w.deltaY, w.deltaMode, WheelEvent.DOM_DELTA_PAGE, w instanceof MouseEvent,
         k.key, k.code, k.location, k.repeat, KeyboardEvent.DOM_KEY_LOCATION_NUMPAD,
         p.pointerId, p.pointerType, p.width, p.clientX, p instanceof MouseEvent,
         p.getCoalescedEvents().length, h.oldURL, h.newURL, cm.data, i.data, i.inputType,
         i.isComposing, s.key, s.url, s.newValue].join('|')"#,
    );
    assert_eq!(
        r,
        "true|7|true|true|\
         1.5|2|300|1|true|true|\
         true|false|true|\
         true|2|\
         3.5|1|2|true|\
         a|KeyA|2|true|3|\
         4|pen|3|9|true|\
         0|a|b|d|x|insertText|\
         true|k|u|"
    );
}

#[test]
fn dictionary_members_are_read_in_order() {
    // WebIDL §3.2.18: inherited dictionaries first, each member lexicographic.
    let r = eval(
        r#"const order = [];
        const init = new Proxy({}, { get(t, k) { if (typeof k === 'string') order.push(k); } });
        new UIEvent('x', init);
        const nulls = [null, undefined].map((v) => new UIEvent('x', v).detail).join();
        let threw = [];
        for (const f of [() => new UIEvent('x', 1), () => new UIEvent('x', { view: {} }),
          () => new MouseEvent('x', { clientX: NaN }), () => new UIEvent('x', { detail: Symbol() }),
          () => new PromiseRejectionEvent('x'), () => new PromiseRejectionEvent('x', {}),
          () => new TouchEvent('x', { touches: 1 }), () => new TouchEvent('x', { touches: [1] })]) {
          try { f(); threw.push('no'); } catch (e) { threw.push(e.name); }
        }
        const p = {};
        [order.join(), nulls, threw.join(),
         new PromiseRejectionEvent('x', { promise: p, reason: 2 }).promise === p].join('|')"#,
    );
    assert_eq!(
        r,
        "bubbles,cancelable,composed,detail,view,which|0,0|TypeError,TypeError,TypeError,TypeError,\
         TypeError,TypeError,TypeError,TypeError|true"
    );
}

#[test]
fn create_event_gives_uninitialized_events() {
    // DOM §4.5; WPT dom/nodes/Document-createEvent.https.html.
    let r = eval(
        r#"const out = [];
        for (const [n, i] of [['Event', Event], ['EVENTS', Event], ['htmlevents', Event],
          ['SVGEvents', Event], ['UIEvents', UIEvent], ['MouseEvents', MouseEvent],
          ['customevent', CustomEvent], ['KeyboardEvent', KeyboardEvent], ['TextEvent', TextEvent],
          ['BeforeUnloadEvent', BeforeUnloadEvent], ['DragEvent', DragEvent],
          ['MessageEvent', MessageEvent], ['StorageEvent', StorageEvent], ['TouchEvent', TouchEvent],
          ['HashChangeEvent', HashChangeEvent], ['DeviceMotionEvent', DeviceMotionEvent]]) {
          const e = document.createEvent(n);
          if (!(e instanceof i) || e.type !== '' || e.bubbles || e.isTrusted) out.push('bad ' + n);
        }
        for (const n of ['', 'Eventx', 'PointerEvent', 'WheelEvent', 'SubmitEvent']) {
          try { document.createEvent(n); out.push('no ' + n); }
          catch (e) { out.push(e.name === 'NotSupportedError' && e.code === 9); }
        }
        const e = document.createEvent('Event');
        try { a.dispatchEvent(e); out.push('dispatched'); } catch (x) { out.push(x.name); }
        e.initEvent('x', true, false);
        a.addEventListener('x', () => log.push('got'));
        out.push(a.dispatchEvent(e), log.join());
        const m = document.createEvent('MouseEvent');
        m.initMouseEvent('c', true, true, window, 3, 1, 2, 3, 4, true, false, false, true, 2, b);
        out.push(m.type, m.detail, m.clientY, m.ctrlKey, m.metaKey, m.button, m.relatedTarget === b);
        const k = document.createEvent('KeyboardEvent');
        k.initKeyboardEvent('k', false, false, null, 'q', 1);
        const ce = document.createEvent('CustomEvent');
        ce.initCustomEvent('c', false, false, 5);
        const u = document.createEvent('UIEvent');
        u.initUIEvent('u', false, false, window, 9);
        out.push(k.key, k.location, ce.detail, u.view === window, u.detail);
        out.join('|')"#,
    );
    assert_eq!(
        r,
        "true|true|true|true|true|InvalidStateError|true|got|c|3|4|true|true|2|true|q|1|5|true|9"
    );
}

#[test]
fn a_touch_or_wheel_listener_is_passive_by_default_on_the_root() {
    // DOM §2.7.3 "default passive value"; WPT dom/events/passive-by-default.html.
    let r = eval(
        r#"const out = [];
        const body = document.body;
        for (const t of [window, document, document.documentElement, body, a]) {
          for (const type of ['touchstart', 'wheel', 'mousewheel', 'touchmove', 'touchend']) {
            for (const passive of ['omitted', undefined, false, true]) {
              const f = (e) => { e.preventDefault(); out.push(e.defaultPrevented ? 1 : 0); };
              t.addEventListener(type, f, passive === 'omitted' ? undefined : { passive });
              t.dispatchEvent(new Event(type, { cancelable: true }));
              t.removeEventListener(type, f);
            }
          }
        }
        out.join('')"#,
    );
    // Per target: four event types of 0 0 1 0, and touchend of 1 1 1 0; the
    // div is 1 1 1 0 for all five.
    let root = format!("{}1110", "0010".repeat(4));
    assert_eq!(r, format!("{}{}", root.repeat(4), "1110".repeat(5)));
}

#[test]
fn hostile_dictionaries_and_event_names() {
    // A getter that throws stops the conversion; a getter can't reach the
    // record, and a replaced prototype changes nothing here.
    let r = eval(
        r#"let thrown = '';
        try { new MouseEvent('x', { get altKey() { throw new RangeError('mine'); } }); }
        catch (e) { thrown = e.name; }
        Object.defineProperty(Object.prototype, 'bubbles', { get() { return true; }, configurable: true });
        Object.defineProperty(Array.prototype, 0, { set() { throw 1; }, configurable: true });
        const e = document.createEvent('Event');
        const inherited = new Event('x', {}).bubbles;
        const sequence = new PointerEvent('x').getCoalescedEvents().length;
        delete Object.prototype.bubbles;
        delete Array.prototype[0];
        const held = new CustomEvent('x', { detail: e });
        [thrown, inherited, e.bubbles, sequence, held.detail === e].join('|')"#,
    );
    assert_eq!(r, "RangeError|true|false|0|true");
    // An endless sequence of what can't be converted stops at the first
    // element; of what can be, it ends at a limit, not a hang.
    let r = eval(
        "try { new TouchEvent('x', { touches: { [Symbol.iterator]() { return { next() {
           return { done: false, value: 1 }; } }; } } }); 'no' } catch (e) { e.name }",
    );
    assert_eq!(r, "TypeError");
}

#[test]
fn window_event_is_the_running_listeners_event() {
    // DOM §2.9 "inner invoke"; HTML §7.2; WPT dom/events/event-global.html.
    let r = eval(
        r#"const own = Object.getOwnPropertyDescriptor(window, 'event') !== undefined;
        const before = window.event;
        let inner, outer, after;
        const e = new Event('x'), f = new Event('y');
        b.addEventListener('y', () => { inner = window.event === f; });
        a.addEventListener('x', () => {
          b.dispatchEvent(f);
          outer = window.event === e;
          throw new Error('no');
        });
        addEventListener('error', (x) => { x.preventDefault(); log.push(window.event.type); });
        a.dispatchEvent(e);
        after = window.event;
        Object.prototype.get = () => 7;
        window.event = 5;
        delete Object.prototype.get;
        [own, before, inner, outer, after, log.join(), window.event].join('|')"#,
    );
    assert_eq!(r, "true||true|true||error|5");
}

#[test]
fn is_trusted_cant_be_replaced() {
    // DOM §2.2, [LegacyUnforgeable]; WPT dom/events/Event-isTrusted.any.js.
    let r = eval(
        r#"const e = new Event('x'), f = new Event('y');
        const d = Object.getOwnPropertyDescriptor(e, 'isTrusted');
        const g = Object.getOwnPropertyDescriptor(f, 'isTrusted');
        let threw = '';
        try { Object.defineProperty(e, 'isTrusted', { value: true }); } catch (x) { threw = x.name; }
        [typeof d.get, d.get === g.get, d.configurable, d.set, e.isTrusted, threw,
         'isTrusted' in Event.prototype].join('|')"#,
    );
    assert_eq!(r, "function|true|false||false|TypeError|false");
}

#[test]
fn a_signal_removes_its_listeners() {
    // DOM §2.7.3, §3.2; WPT dom/events/AddEventListenerOptions-signal.any.js.
    let r = eval(
        r#"const ctl = new AbortController();
        let n = 0;
        const f = () => { n++; };
        a.addEventListener('x', f, { signal: ctl.signal });
        a.dispatchEvent(new Event('x'));
        ctl.abort();
        a.dispatchEvent(new Event('x'));
        a.addEventListener('x', f, { signal: ctl.signal });
        a.dispatchEvent(new Event('x'));
        let threw = '';
        try { a.addEventListener('x', f, { signal: null }); } catch (e) { threw = e.name; }
        const s = AbortSignal.abort('why');
        const d = AbortSignal.any([new AbortController().signal, ctl.signal]);
        const c2 = new AbortController(), d2 = AbortSignal.any([c2.signal]);
        d2.addEventListener('abort', () => log.push('abort'));
        ctl.signal.onabort = () => log.push('late');
        c2.abort();
        let reason = '';
        try { ctl.signal.throwIfAborted(); } catch (e) { reason = e.name; }
        [n, threw, ctl.signal.aborted, s.reason, d.aborted, d.reason === ctl.signal.reason,
         d2.aborted, d2.reason.name, reason, log.join(), s instanceof EventTarget].join('|')"#,
    );
    assert_eq!(
        r,
        "1|TypeError|true|why|true|true|true|AbortError|AbortError|abort|true"
    );
}

#[test]
fn abort_signal_timeout_is_on_the_virtual_clock() {
    // DOM §3.2; the reason's name; settles without waiting.
    let (mut rt, _) = page(
        "<!doctype html><script>globalThis.log = [];
         const s = AbortSignal.timeout(5000);
         s.addEventListener('abort', () => log.push(s.reason.name, performance.now()));
         </script>",
    );
    assert_eq!(log_of(&mut rt), "TimeoutError,5000");
}

#[test]
fn checkbox_and_radio_activation() {
    // HTML §4.10.5.1.15, §6.6; WPT dom/events/Event-dispatch-click.html.
    let r = eval(
        r#"const box = document.createElement('input');
        box.type = 'checkbox';
        a.append(box);
        box.addEventListener('click', () => log.push('click ' + box.checked));
        box.addEventListener('input', () => log.push('input'));
        box.addEventListener('change', () => log.push('change'));
        box.click();
        box.dispatchEvent(new Event('click'));
        box.indeterminate = true;
        const cancel = (e) => { e.preventDefault(); };
        box.addEventListener('click', cancel);
        box.click();
        log.push(box.checked, box.indeterminate);
        const r1 = document.createElement('input'), r2 = document.createElement('input');
        r1.type = r2.type = 'radio';
        r1.name = r2.name = 'n';
        a.append(r1, r2);
        r1.click();
        r2.click();
        log.push(r1.checked, r2.checked);
        r1.addEventListener('click', cancel);
        r1.click();
        log.push(r1.checked, r2.checked);
        const off = document.createElement('input');
        off.type = 'checkbox';
        off.disabled = true;
        a.append(off);
        off.click();
        log.push(off.checked);
        log.join('|')"#,
    );
    // The first click checks; a click that isn't a MouseEvent has no
    // activation behaviour; the cancelled one is undone after its listeners.
    assert_eq!(
        r,
        "click true|input|change|click true|click false|true|true|false|true|false|true|false"
    );
}

#[test]
fn forms_fire_their_events_and_go_nowhere() {
    // HTML §4.10.21.3, §4.10.22; WPT dom/events/Event-dispatch-click.html.
    let r = eval(
        r#"a.innerHTML = '<form id=f><input id=s type=submit><button id=b></button>' +
          '<button id=r type=reset></button><button id=p type=button></button>' +
          '<fieldset disabled><input id=d type=submit></fieldset></form>' +
          '<input id=o type=submit form=f><input id=x type=submit>';
        const $ = (i) => document.getElementById(i);
        const f = $('f'), inside = $('b');
        f.addEventListener('submit', (e) => {
          log.push('submit ' + (e.submitter && e.submitter.id) + ' ' + e.isTrusted + ' ' + e.cancelable);
        });
        f.addEventListener('reset', () => log.push('reset'));
        $('s').click(); $('b').click(); $('r').click(); $('p').click(); $('d').click();
        $('o').click(); $('x').click();
        f.requestSubmit();
        f.requestSubmit($('s'));
        let threw = [];
        try { f.requestSubmit($('p')); } catch (e) { threw.push(e.name); }
        try { f.requestSubmit($('x')); } catch (e) { threw.push(e.name); }
        f.submit();
        const owners = [$('s').form === f, $('o').form === f, $('x').form];
        f.remove();
        inside.click();
        log.push(threw.join(), ...owners, inside.type, inside.form === f, inside.isConnected);
        log.join('|')"#,
    );
    assert_eq!(
        r,
        "submit s true true|submit b true true|reset|submit o true true|\
         submit null true true|submit s true true|TypeError,NotFoundError|true|true||submit|true|false"
    );
}

#[test]
fn labels_and_details() {
    // HTML §4.10.4, §4.11.1; WPT dom/events/label-default-action.html.
    let (mut rt, _) = page(
        "<!doctype html><label id=l for=c>t<b id=t>x</b></label>
         <input id=c type=checkbox>
         <label id=m><button id=i></button><input id=n type=checkbox></label>
         <details id=d><summary id=s>x</summary><summary id=z>y</summary></details>
         <script>globalThis.log = [];
         const $ = (i) => document.getElementById(i);
         $('t').click(); $('i').click(); $('s').click();
         log.push($('c').checked, $('n').checked, $('d').open, $('l').control === $('c'),
           $('m').control === $('i'), $('l').htmlFor);
         $('z').click();
         log.push($('d').open);
         $('d').addEventListener('toggle', (e) => log.push(e.oldState, e.newState, e.isTrusted));
         </script>",
    );
    assert_eq!(
        log_of(&mut rt),
        "true,false,true,true,true,c,true,closed,open,true"
    );
}

#[test]
fn a_javascript_link_runs_in_a_task() {
    // HTML §4.6.4; §7.11 for javascript: URLs. WPT Event-dispatch-click.html.
    let (mut rt, report) = page(
        "<!doctype html><a id=a href='javascript:log.push(%22link%22)'>x</a>
         <a id=b href='javascript:nope('>y</a><a id=c>z</a><a id=d href='#f'>w</a>
         <script>globalThis.log = [];
         const $ = (i) => document.getElementById(i);
         $('a').click(); log.push('after');
         $('c').click(); $('d').click(); $('b').click();
         </script>",
    );
    assert_eq!(log_of(&mut rt), "after,link");
    assert_eq!(report.uncaught, 1, "{:?}", report.errors);
}

#[test]
fn a_limit_in_a_javascript_link_ends_the_page() {
    // The link's task runs under the same time limit as any other.
    let mut rt = DomRuntime::new(limits().with_time(common::scaled(Duration::from_millis(200))))
        .unwrap();
    rt.load(
        Document::parse(
            "<!doctype html><a id=a href='javascript:for(;;);'>x</a>
             <script>document.getElementById('a').click();</script>",
        )
        .unwrap(),
    );
    let e = rt.run_page(&SettleOptions::default(), |_| None).unwrap_err();
    assert!(
        matches!(e, DomError::Sandbox(SandboxError::TimeLimit { .. })),
        "{e:?}"
    );
}

#[test]
fn clicks_cant_recurse_or_be_forged() {
    // A click in progress on an element does nothing (HTML §6.6), so this ends;
    // a form's reset and submit are guarded the same way.
    let r = eval(
        r#"let n = 0;
        a.addEventListener('click', () => { n++; a.click(); });
        a.click();
        a.innerHTML = '<form id=f><button id=b></button></form>';
        const f = document.getElementById('f'), b2 = document.getElementById('b');
        let m = 0;
        f.addEventListener('submit', () => { m++; f.requestSubmit(); });
        b2.click();
        let k = 0;
        f.addEventListener('reset', () => { k++; f.reset(); });
        f.reset();
        const fake = Object.create(HTMLElement.prototype);
        let threw = '';
        try { fake.click(); } catch (e) { threw = e.name; }
        [n, m, k, threw].join('|')"#,
    );
    // The click on the button bubbles to `a`, whose listener then clicks `a`.
    assert_eq!(r, "3|1|1|TypeError");
}

#[test]
fn elements_are_named_on_the_window() {
    // HTML §7.3.3; WebIDL §3.7.4: a real property hides a name, and a global
    // that isn't there is still a ReferenceError.
    let (mut rt, _) = page(
        "<!doctype html><div id=one></div><p id=window></p><p id=setTimeout></p><i id=two></i>
         <script>globalThis.log = [];
         log.push(one === document.getElementById('one'), typeof setTimeout, window === self);
         let threw = '';
         try { nothing_here; } catch (e) { threw = e.name; }
         log.push(threw, typeof nothing_here, 'one' in window);
         document.getElementById('two').remove();
         log.push(typeof two);
         </script><script>var one = 5; log.push(window.one);</script>",
    );
    assert_eq!(
        log_of(&mut rt),
        "true,function,true,ReferenceError,undefined,true,undefined,5"
    );
}
