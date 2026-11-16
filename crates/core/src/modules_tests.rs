//! SPIKE (P2-RT-12) measurements of QuickJS-ng modules under the sandbox.
//! Run with `--nocapture` to see the numbers; `#[ignore]` ones are the long
//! measurements (`-- --ignored p2_rt_12`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::{
    Limits, Sandbox, SandboxError, SandboxOptions, SettleOptions, SpikeModuleHost,
    SpikeModuleLimits, modules::spike_declare_module, spike_eval_module,
};

/// URL-ish resolution good enough for tests: relative specifiers against the
/// referrer's directory, `/x` against the origin, bare ones through `map`.
fn resolver(map: HashMap<String, String>) -> crate::SpikeResolve {
    Box::new(move |base: &str, spec: &str| {
        if let Some(u) = map.get(spec) {
            return Ok(u.clone());
        }
        if spec.starts_with("https://") {
            return Ok(spec.to_owned());
        }
        if spec.starts_with('/') {
            return Ok(format!("https://app.test{spec}"));
        }
        if spec.starts_with("./") || spec.starts_with("../") {
            let dir = base.rsplit_once('/').map_or("https://app.test", |(d, _)| d);
            let mut parts: Vec<&str> = dir.split('/').collect();
            for seg in spec.split('/') {
                match seg {
                    "." => {}
                    ".." => {
                        if parts.len() > 3 {
                            parts.pop();
                        }
                    }
                    s => parts.push(s),
                }
            }
            return Ok(parts.join("/"));
        }
        Err(format!(
            "Failed to resolve module specifier \"{spec}\": bare specifier not in the import map"
        ))
    })
}

type Files = Rc<RefCell<HashMap<String, String>>>;

fn sandbox_with(files: &[(&str, &str)], limits: Limits) -> (Sandbox, Files) {
    let mut sb = Sandbox::with_options(SandboxOptions::default().with_limits(limits)).unwrap();
    let store: Files = Rc::new(RefCell::new(
        files
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    ));
    let s = store.clone();
    sb.spike_set_module_host(SpikeModuleHost::new(
        resolver(HashMap::new()),
        Box::new(move |url| s.borrow().get(url).cloned()),
    ));
    (sb, store)
}

/// Runs `src` as an inline module of https://app.test/page.html, drains
/// microtasks, and returns `globalThis.out` as a string.
fn run_entry(sb: &mut Sandbox, src: &str) -> Result<String, SandboxError> {
    // Each entry needs its own name: the engine finds a module, and its
    // `import.meta`, by name.
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let name = format!(
        "https://app.test/page.html#inline-{}",
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    sb.run_await(
        |ctx| {
            spike_eval_module(ctx, &name, "https://app.test/page.html", src).map(|p| p.into_value())
        },
        |_, _| Ok(()),
    )?;
    sb.eval::<String>("String(globalThis.out)")
}

#[test]
fn p2_rt_12_static_imports_meta_once_cycles() {
    let (mut sb, _) = sandbox_with(
        &[
            (
                "https://app.test/a.js",
                "import { b } from './b.js'; globalThis.evals = (globalThis.evals|0) + 1; export const a = 'a' + b; export const meta = import.meta.url;",
            ),
            (
                "https://app.test/b.js",
                "import { a } from './a.js'; export const b = 'b'; export function late() { return a; }",
            ),
            (
                "https://app.test/lib/c.js",
                "export default 42; export const self = this; export const strict = (function(){ return this === undefined })();",
            ),
        ],
        Limits::default(),
    );
    let out = run_entry(
        &mut sb,
        "import { a, meta } from './a.js'; import { late } from './b.js'; import * as ns from './a.js'; \
         import c, { self, strict } from '/lib/c.js'; \
         globalThis.out = [a, meta, late(), Object.keys(ns).join(), ns[Symbol.toStringTag], c, String(self), strict, String(this), import.meta.url, globalThis.evals].join('|');",
    )
    .unwrap();
    println!("static: {out}");
    assert_eq!(
        out,
        "ab|https://app.test/a.js|ab|a,meta|Module|42|undefined|true|undefined|https://app.test/page.html|1"
    );
}

#[test]
fn p2_rt_12_dynamic_import_and_tla() {
    let (mut sb, _) = sandbox_with(
        &[
            (
                "https://app.test/d.js",
                "export const v = await Promise.resolve('tla');",
            ),
            ("https://app.test/e.js", "export const e = 'e';"),
        ],
        Limits::default(),
    );
    let out = run_entry(
        &mut sb,
        "const m = await import('./d.js'); const again = await import('./d.js'); \
         globalThis.out = m.v + (m === again);",
    )
    .unwrap();
    assert_eq!(out, "tlatrue");
    // import() from a classic script: the referrer is the script's file name.
    let v: String = sb
        .run_await(
            |ctx| ctx.eval::<rquickjs::Value, _>("import('https://app.test/e.js').then(m => m.e)"),
            |_, v| Ok(v.as_string().unwrap().to_string().unwrap()),
        )
        .unwrap();
    assert_eq!(v, "e");
}

#[test]
fn p2_rt_12_errors() {
    let (mut sb, _) = sandbox_with(
        &[
            ("https://app.test/syntax.js", "export const = 1;"),
            ("https://app.test/noexport.js", "export const y = 1;"),
            (
                "https://app.test/throws.js",
                "throw new Error('boom in eval');",
            ),
            (
                "https://app.test/ok.js",
                "globalThis.okRan = true; export {};",
            ),
        ],
        Limits::default(),
    );
    for (label, src) in [
        ("missing", "import './nope.js';"),
        ("bare", "import 'lodash';"),
        ("syntax-dep", "import './ok.js'; import './syntax.js';"),
        ("link", "import { x } from './noexport.js';"),
        ("eval-dep", "import './throws.js';"),
        ("syntax-entry", "export default ;"),
        ("json", "import j from './x.json' with { type: 'json' };"),
        (
            "dyn-missing",
            "globalThis.out = await import('./nope2.js').then(() => 'loaded', e => 'rejected: ' + e);",
        ),
    ] {
        let r = run_entry(&mut sb, src);
        let ok_ran: String = sb.eval("String(globalThis.okRan)").unwrap();
        println!("error case {label}: {r:?} okRan={ok_ran}");
    }
}

#[test]
fn p2_rt_12_tla_never_settles() {
    // Microtasks only: a pending TLA is a Type error, never a hang.
    let (mut sb, _) = sandbox_with(&[], Limits::default());
    let r = run_entry(
        &mut sb,
        "await new Promise(() => {}); globalThis.out = 'after';",
    );
    println!("tla pending, run_await: {r:?}");
    assert!(matches!(r, Err(SandboxError::Type { .. })), "{r:?}");
    // On the event loop: the loop settles (idle) with the module still pending.
    let mut sb = Sandbox::with_options(SandboxOptions::default().with_event_loop(true)).unwrap();
    sb.spike_set_module_host(SpikeModuleHost::new(
        resolver(HashMap::new()),
        Box::new(|_| None),
    ));
    sb.run(|ctx| {
        let p = spike_eval_module(
            ctx,
            "https://app.test/page.html#0",
            "https://app.test/page.html",
            "await new Promise(r => setTimeout(r, 50)); globalThis.out = 'timer'; await new Promise(() => {}); globalThis.out2 = 'never';",
        )?;
        ctx.globals().set("entry", p)?;
        Ok(())
    })
    .unwrap();
    let t = Instant::now();
    let report = sb.run_until_settled(&SettleOptions::default());
    let out: String = sb
        .eval("String(globalThis.out) + '/' + String(globalThis.out2)")
        .unwrap();
    println!(
        "tla on event loop: {:?} in {:?}; {out}",
        report.map(|r| r.outcome),
        t.elapsed()
    );
    assert_eq!(out, "timer/undefined");
}

/// The engine's limits on module graphs, each with its own sandbox.
#[test]
fn p2_rt_12_limits_on_graphs() {
    let roomy = SpikeModuleLimits {
        max_modules: 100_000,
        max_bytes: 1 << 30,
    };
    let cases: Vec<(
        &str,
        Box<dyn Fn(&mut HashMap<String, String>) -> String>,
        Limits,
    )> = vec![
        (
            "deep chain 10,000",
            Box::new(|f: &mut HashMap<String, String>| {
                for i in 0..10_000 {
                    let next = if i + 1 < 10_000 {
                        format!("import './m{}.js'; ", i + 1)
                    } else {
                        String::new()
                    };
                    f.insert(
                        format!("https://app.test/m{i}.js"),
                        format!("{next}globalThis.n = (globalThis.n|0) + 1;"),
                    );
                }
                "import './m0.js'; globalThis.out = globalThis.n;".to_owned()
            }),
            Limits::default(),
        ),
        (
            "deep chain 100,000",
            Box::new(|f: &mut HashMap<String, String>| {
                for i in 0..100_000 {
                    let next = if i + 1 < 100_000 {
                        format!("import './m{}.js'; ", i + 1)
                    } else {
                        String::new()
                    };
                    f.insert(
                        format!("https://app.test/m{i}.js"),
                        format!("{next}globalThis.n = (globalThis.n|0) + 1;"),
                    );
                }
                "import './m0.js'; globalThis.out = globalThis.n;".to_owned()
            }),
            Limits::default()
                .with_time(Duration::from_secs(60))
                .with_memory_bytes(1 << 30),
        ),
        (
            "wide 10,000",
            Box::new(|f: &mut HashMap<String, String>| {
                let mut entry = String::new();
                for i in 0..10_000 {
                    f.insert(
                        format!("https://app.test/w{i}.js"),
                        "globalThis.n = (globalThis.n|0) + 1;".to_owned(),
                    );
                    entry.push_str(&format!("import './w{i}.js';\n"));
                }
                entry.push_str("globalThis.out = globalThis.n;");
                entry
            }),
            Limits::default(),
        ),
        (
            "ring of 10,000",
            Box::new(|f: &mut HashMap<String, String>| {
                for i in 0..10_000 {
                    f.insert(
                        format!("https://app.test/r{i}.js"),
                        format!("import {{ v as w }} from './r{}.js'; export const v = {i}; export function get() {{ return w; }} globalThis.n = (globalThis.n|0) + 1;", (i + 1) % 10_000),
                    );
                }
                "import { get } from './r0.js'; globalThis.out = globalThis.n + ':' + get();"
                    .to_owned()
            }),
            Limits::default(),
        ),
        (
            "50 MB module (64 MB heap)",
            Box::new(|f: &mut HashMap<String, String>| {
                let mut s = String::with_capacity(50 << 20);
                s.push_str("export const big = [");
                while s.len() < (50 << 20) {
                    s.push_str("1234567,");
                }
                s.push_str("0];");
                f.insert("https://app.test/big.js".into(), s);
                "import { big } from './big.js'; globalThis.out = big.length;".to_owned()
            }),
            Limits::default(),
        ),
        (
            "50 MB module (1 GiB heap, 2 s)",
            Box::new(|f: &mut HashMap<String, String>| {
                let mut s = String::with_capacity(50 << 20);
                s.push_str("export const big = [");
                while s.len() < (50 << 20) {
                    s.push_str("1234567,");
                }
                s.push_str("0];");
                f.insert("https://app.test/big.js".into(), s);
                "import { big } from './big.js'; globalThis.out = big.length;".to_owned()
            }),
            Limits::default().with_memory_bytes(1 << 30),
        ),
        (
            "endless loop in a dependency",
            Box::new(|f: &mut HashMap<String, String>| {
                f.insert("https://app.test/loop.js".into(), "for (;;) {}".into());
                "import './loop.js';".to_owned()
            }),
            Limits::default().with_time(Duration::from_millis(200)),
        ),
        (
            "endless loop after await",
            Box::new(|f: &mut HashMap<String, String>| {
                f.insert(
                    "https://app.test/loop.js".into(),
                    "await 0; for (;;) {}".into(),
                );
                "import './loop.js';".to_owned()
            }),
            Limits::default().with_time(Duration::from_millis(200)),
        ),
        (
            "recursion in a module",
            Box::new(|_f: &mut HashMap<String, String>| {
                "function r() { return r() + 1; } r();".to_owned()
            }),
            Limits::default(),
        ),
        (
            "memory bomb in a module",
            Box::new(|_f: &mut HashMap<String, String>| {
                "const a = []; for (;;) a.push(new Array(1000).fill(1));".to_owned()
            }),
            Limits::default(),
        ),
        (
            "import() flood",
            Box::new(|f: &mut HashMap<String, String>| {
                f.insert("https://app.test/x.js".into(), "export {};".into());
                "for (let i = 0; i < 1e7; i++) import('./x.js?' + i);".to_owned()
            }),
            Limits::default(),
        ),
    ];
    for (label, build, limits) in cases {
        let mut files = HashMap::new();
        let entry = build(&mut files);
        let mut sb = Sandbox::with_options(SandboxOptions::default().with_limits(limits)).unwrap();
        let fetches = Rc::new(RefCell::new(0usize));
        let fc = fetches.clone();
        let mut host = SpikeModuleHost::new(
            resolver(HashMap::new()),
            Box::new(move |url| {
                *fc.borrow_mut() += 1;
                let key = url.split('?').next().unwrap_or(url);
                files.get(key).cloned()
            }),
        );
        host.limits = roomy.clone();
        sb.spike_set_module_host(host);
        let t = Instant::now();
        let r = run_entry(&mut sb, &entry);
        println!(
            "limit case {label}: {r:?} in {:?}, {} fetches, rss {} MB",
            t.elapsed(),
            fetches.borrow(),
            rss_mb()
        );
        // The sandbox goes on after a limit.
        assert_eq!(sb.eval::<i32>("6*7").unwrap(), 42);
    }
}

/// A fetch that blocks: the watchdog can't stop it, the next poll after it can.
#[test]
fn p2_rt_12_slow_fetch() {
    let mut sb = Sandbox::with_options(
        SandboxOptions::default()
            .with_limits(Limits::default().with_time(Duration::from_millis(100))),
    )
    .unwrap();
    sb.spike_set_module_host(SpikeModuleHost::new(
        resolver(HashMap::new()),
        Box::new(|_| {
            std::thread::sleep(Duration::from_millis(300));
            Some("export const v = 1; for (let i = 0; i < 1e6; i++);".into())
        }),
    ));
    let t = Instant::now();
    let r = run_entry(&mut sb, "import './slow.js'; import './slow2.js';");
    println!("slow fetch: {r:?} in {:?}", t.elapsed());
}

/// The bound on module count and bytes, in the host.
#[test]
fn p2_rt_12_host_bounds() {
    let mut sb = Sandbox::new(Limits::default()).unwrap();
    let mut host = SpikeModuleHost::new(
        resolver(HashMap::new()),
        Box::new(|url| {
            let i: usize = url
                .trim_start_matches("https://app.test/n")
                .trim_end_matches(".js")
                .parse()
                .unwrap_or(0);
            Some(format!("import './n{}.js';", i + 1))
        }),
    );
    host.limits.max_modules = 100;
    let shared = sb.spike_set_module_host(host);
    let r = run_entry(&mut sb, "import './n0.js';");
    println!("host bound: {r:?}, loaded {}", shared.borrow().loaded.len());
    assert!(matches!(r, Err(SandboxError::Exception { .. })));
}

fn rss_mb() -> u64 {
    let s = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: u64 = s
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    pages * 4096 / (1 << 20)
}

fn graph(n: usize) -> (HashMap<String, String>, String, String) {
    // n modules, each importing up to 3 earlier ones, each defining a few
    // functions; plus the same code as one classic bundle.
    let mut files = HashMap::new();
    let mut bundle = String::from("(function(){ var M = {};\n");
    for i in 0..n {
        let mut src = String::new();
        let mut deps = Vec::new();
        for d in [i / 2, i / 3, i.saturating_sub(1)] {
            if d < i && !deps.contains(&d) {
                deps.push(d);
            }
        }
        for d in &deps {
            src.push_str(&format!("import {{ f{d} }} from './g{d}.js';\n"));
        }
        let body = format!(
            "function f{i}(x) {{ let s = x; for (let k = 0; k < 3; k++) s += k * {i}; return s{}; }}\n\
             const table{i} = Array.from({{ length: 20 }}, (_, k) => ({{ k, name: 'item' + k + '_{i}' }}));\n\
             class C{i} {{ constructor(v) {{ this.v = v; }} get twice() {{ return this.v * 2; }} }}\n",
            deps.iter()
                .map(|d| format!(" + (typeof f{d} === 'function' ? 1 : 0)"))
                .collect::<String>()
        );
        src.push_str(&body);
        src.push_str(&format!("export {{ f{i}, table{i}, C{i} }};\n"));
        files.insert(format!("https://app.test/g{i}.js"), src);
        bundle.push_str(&body);
    }
    bundle.push_str(&format!("globalThis.out = f{}(1); }})();", n - 1));
    let entry = format!(
        "import {{ f{0} }} from './g{0}.js'; globalThis.out = f{0}(1);",
        n - 1
    );
    (files, entry, bundle)
}

#[test]
#[ignore = "measurement"]
fn p2_rt_12_cost_300_modules_vs_bundle() {
    let (files, entry, bundle) = graph(300);
    let module_bytes: usize = files.values().map(String::len).sum();
    println!(
        "300-module graph: {module_bytes} bytes; bundle {} bytes",
        bundle.len()
    );
    for round in 0..3 {
        let mut sb = Sandbox::new(Limits::default()).unwrap();
        let f = files.clone();
        sb.spike_set_module_host(SpikeModuleHost::new(
            resolver(HashMap::new()),
            Box::new(move |url| f.get(url).cloned()),
        ));
        let base = sb.heap_bytes();
        let t = Instant::now();
        let a = run_entry(&mut sb, &entry).unwrap();
        let tm = t.elapsed();
        let heap_m = sb.heap_bytes() - base;
        let mut sb2 = Sandbox::new(Limits::default()).unwrap();
        let base2 = sb2.heap_bytes();
        let t = Instant::now();
        sb2.eval::<()>(&bundle).unwrap();
        let tb = t.elapsed();
        let heap_b = sb2.heap_bytes() - base2;
        let b: String = sb2.eval("String(globalThis.out)").unwrap();
        assert_eq!(a, b);
        println!(
            "round {round}: modules {tm:?} heap +{} KiB; bundle {tb:?} heap +{} KiB",
            heap_m / 1024,
            heap_b / 1024
        );
    }
}

#[test]
#[ignore = "measurement"]
fn p2_rt_12_soak_2000_renders() {
    let (files, entry, _) = graph(50);
    let mut sb = Sandbox::new(Limits::default()).unwrap();
    let f = files.clone();
    let shared = sb.spike_set_module_host(SpikeModuleHost::new(
        resolver(HashMap::new()),
        Box::new(move |url| f.get(url).cloned()),
    ));
    let t = Instant::now();
    for i in 0..2000 {
        shared.borrow_mut().reset_counters();
        run_entry(&mut sb, &entry).unwrap();
        // Also a module that fails, and one dynamic import.
        let _ = run_entry(&mut sb, "import './missing.js';");
        run_entry(&mut sb, "globalThis.out = (await import('./g3.js')).f3(1);").unwrap();
        sb.reset().unwrap();
        if i % 250 == 0 || i == 1999 {
            println!(
                "render {i}: rss {} MB, heap {} KiB, {:?}",
                rss_mb(),
                sb.heap_bytes() / 1024,
                t.elapsed()
            );
        }
    }
}

/// Modules declared up front (a prefetching loader, a bytecode cache): the
/// engine's resolve walk (`js_resolve_module`) recurses with no stack check.
#[test]
#[ignore = "measurement; may crash the process"]
fn p2_rt_12_predeclared_chain() {
    let n: usize = std::env::var("P2_RT_12_CHAIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    let child = std::thread::Builder::new()
        .stack_size(
            std::env::var("P2_RT_12_THREAD_STACK")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(Limits::DEFAULT_STACK_BYTES + 8 * 1024 * 1024),
        )
        .spawn(move || {
            let mut sb = Sandbox::new(
                Limits::default()
                    .with_memory_bytes(1 << 30)
                    .with_time(Duration::from_secs(300)),
            )
            .unwrap();
            let mut host = SpikeModuleHost::new(resolver(HashMap::new()), Box::new(|_| None));
            host.limits.max_modules = usize::MAX;
            sb.spike_set_module_host(host);
            let t = Instant::now();
            sb.run(|ctx| {
                for i in (0..n).rev() {
                    let next = if i + 1 < n {
                        format!("import './p{}.js'; ", i + 1)
                    } else {
                        String::new()
                    };
                    spike_declare_module(
                        ctx,
                        &format!("https://app.test/p{i}.js"),
                        &format!("{next}globalThis.n = (globalThis.n|0) + 1;"),
                    )?;
                }
                Ok(())
            })
            .unwrap();
            println!("declared {n} in {:?}", t.elapsed());
            let t = Instant::now();
            let r = run_entry(&mut sb, "import './p0.js'; globalThis.out = globalThis.n;");
            println!("predeclared chain {n}: {r:?} in {:?}", t.elapsed());
        })
        .unwrap();
    child.join().unwrap();
}

/// A module whose compile fails while its graph resolves is freed
/// (`__JS_EvalInternal`, `fail1`), but a module loaded earlier in the same
/// walk that imports it back (a cycle) keeps a pointer to it. A later graph
/// through that module links freed memory. From WPT
/// `html/semantics/scripting-1/the-script-element/module/nested-imports.html`.
#[test]
#[ignore = "use after free on an unpatched engine"]
fn p2_rt_12_failed_graph_then_reuse() {
    let (mut sb, _) = sandbox_with(
        &[
            (
                "https://app.test/c.js",
                "import './d.js'; export const c = 'c';",
            ),
            (
                "https://app.test/d.js",
                "import './e.js'; import './missing.js'; export const d = 'd';",
            ),
            (
                "https://app.test/e.js",
                "import { c } from './c.js'; export const e = 'e';",
            ),
        ],
        Limits::default(),
    );
    let first = run_entry(&mut sb, "import './c.js';");
    println!("first graph: {first:?}");
    let second = run_entry(&mut sb, "import { e } from './e.js'; globalThis.out = e;");
    println!("second graph: {second:?}");
}
