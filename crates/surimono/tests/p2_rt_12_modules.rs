//! SPIKE (P2-RT-12): module scripts through `render_page`. Run with
//! `--nocapture`; the Rails case needs `P2_RT_12_RAILS=<app dir>` (a
//! precompiled app) and `P2_RT_12_RAILS_PAGE=<rendered html>`.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use surimono::{Options, Renderer, ResourceLoader};

fn opts() -> Options {
    let font =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/fonts/NotoSans-Regular.ttf");
    Options::new().font(std::fs::read(font).unwrap())
}

fn render(html: &str, files: HashMap<String, Vec<u8>>) -> surimono::PageOutput {
    let loader = ResourceLoader::from_map(files);
    let renderer = Renderer::new(opts().resource_loader(loader)).unwrap();
    renderer.render_page(html).unwrap()
}

fn body(html: &str) -> &str {
    let start = html.find("<body").unwrap_or(0);
    &html[start..html.len().min(start + 1500)]
}

#[test]
fn p2_rt_12_order_and_lifecycle() {
    let html = r#"<!doctype html><html><head>
<script>var log = []; document.addEventListener('DOMContentLoaded', () => log.push('DCL')); addEventListener('load', () => { log.push('load'); document.getElementById('o').textContent = log.join(' '); });</script>
<script type="module">log.push('inline-module this=' + this + ' cs=' + document.currentScript + ' meta=' + import.meta.url);</script>
<script type="module" src="/js/a.js"></script>
<script>log.push('classic-after-module');</script>
<script nomodule>log.push('NOMODULE-RAN');</script>
<script type="module" async>log.push('async-module');</script>
<script defer src="/js/d.js"></script>
<script type="module">await Promise.resolve(); log.push('tla-1'); await new Promise(r => setTimeout(r, 0)); log.push('tla-after-timer');</script>
<script type="module">import('/js/dyn.js').then(m => log.push('dyn ' + m.v));</script>
<script type="module">throw new Error('module throws');</script>
<script type="module">import './missing.js'; log.push('NEVER');</script>
<script type="module">log.push('noModule' + ('noModule' in HTMLScriptElement.prototype));</script>
</head><body><p id="o"></p></body></html>"#;
    let files: HashMap<String, Vec<u8>> = [
        (
            "/js/a.js",
            "import { b } from './b.js'; log.push('src-module ' + b + ' ' + import.meta.url);",
        ),
        ("/js/b.js", "export const b = 'b';"),
        ("/js/d.js", "log.push('defer-classic');"),
        ("/js/dyn.js", "export const v = 'v';"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.as_bytes().to_vec()))
    .collect();
    let out = render(html, files);
    println!("order: {}", body(&out.html));
    println!("report: {:?}", out.report);
}

#[test]
fn p2_rt_12_vite_style_page() {
    // What `vite build` writes: one entry chunk with relative imports of
    // hashed chunks, a modulepreload link, and a dynamic import.
    let html = r#"<!doctype html><html><head>
<script type="module" crossorigin src="/assets/index-B1x2.js"></script>
<link rel="modulepreload" crossorigin href="/assets/vendor-C3d4.js">
</head><body><div id="app"></div></body></html>"#;
    let files: HashMap<String, Vec<u8>> = [
        (
            "/assets/index-B1x2.js",
            "import { h, render } from './vendor-C3d4.js';\nconst rows = [['Widget', 3, 9.5], ['Gadget', 1, 120]];\nrender(document.getElementById('app'), h('table', rows.map(r => h('tr', r.map(c => h('td', String(c)))))));\nimport('./chart-D5e6.js').then(m => m.draw(document.getElementById('app')));",
        ),
        (
            "/assets/vendor-C3d4.js",
            "export function h(tag, children) { const el = document.createElement(tag); for (const c of [].concat(children)) el.append(c); return el; }\nexport function render(root, node) { root.replaceChildren(node); }",
        ),
        (
            "/assets/chart-D5e6.js",
            "export function draw(root) { const p = document.createElement('p'); p.textContent = 'chart drawn from ' + new URL(import.meta.url).pathname; root.append(p); }",
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.as_bytes().to_vec()))
    .collect();
    let out = render(html, files);
    println!("vite: {}", body(&out.html));
    println!("report: {:?}", out.report);
    assert!(out.html.contains("<td>Widget</td>"));
    assert!(out.html.contains("chart drawn from /assets/chart-D5e6.js"));
}

#[test]
fn p2_rt_12_rails_importmap_page() {
    let (Ok(app), Ok(page)) = (
        std::env::var("P2_RT_12_RAILS"),
        std::env::var("P2_RT_12_RAILS_PAGE"),
    ) else {
        println!("skipped: P2_RT_12_RAILS and P2_RT_12_RAILS_PAGE not set");
        return;
    };
    let html = std::fs::read_to_string(page).unwrap();
    let public = Path::new(&app).join("public");
    // What `Surimono::Rails.assets` does for `public/`, roughly: a URL under
    // /assets/ is a file there.
    let loader = ResourceLoader::new(["/assets/"], move |url| {
        let path = url.split(['?', '#']).next().unwrap_or(url);
        std::fs::read(public.join(path.trim_start_matches('/'))).map_err(|e| e.to_string())
    });
    let renderer = Renderer::new(opts().resource_loader(loader)).unwrap();
    let t = Instant::now();
    let out = renderer.render_page(&html).unwrap();
    println!("rails: {:?}", t.elapsed());
    let at = out.html.find("<body").unwrap_or(0);
    println!("rails body: {}", &out.html[at..]);
    // What ran: Stimulus connected the controller, Turbo set itself up.
    let probe = r#"<script type="module">
      import "application";
      setTimeout(() => {
        const g = document.getElementById("greet");
        document.body.insertAdjacentHTML("beforeend", "<pre id=probe>" + [
          "greet=" + g.textContent,
          "Turbo=" + typeof window.Turbo,
          "Stimulus=" + typeof window.Stimulus,
          "controllers=" + (window.Stimulus ? window.Stimulus.controllers.length : "-"),
          "turbo-session=" + (window.Turbo && typeof window.Turbo.session),
          "customElements=" + typeof customElements,
          "missing=" + ["fetch","IntersectionObserver","ResizeObserver","MutationObserver","requestIdleCallback","matchMedia","structuredClone","TextEncoder","URLSearchParams","history","navigator","EventSource","WebSocket","AbortController","FormData","Headers","Request","Response","crypto","Intl","CSS","getComputedStyle"].filter(n => typeof window[n] === "undefined").join(","),
          "supports-importmap=" + (HTMLScriptElement.supports ? HTMLScriptElement.supports("importmap") : "no supports()"),
          "import.meta.resolve=" + typeof import.meta.resolve,
          "frame=" + document.getElementById("lazy").outerHTML,
        ].join("\n") + "</pre>");
      }, 100);
    </script>"#;
    let html2 = html
        .replace(r#"<script type="module">import "application"</script>"#, probe)
        .replace("<h1>Invoice</h1>", r#"<h1>Invoice</h1><turbo-frame id="lazy" src="/invoices/1/lines">Loading</turbo-frame>"#);
    let out2 = renderer.render_page(&html2).unwrap();
    let at = out2.html.find("<pre id=\"probe\">").unwrap_or(0);
    println!(
        "rails probe: {}",
        &out2.html[at..out2.html.len().min(at + 600)]
    );
    println!("rails probe report: {:#?}", out2.report);
    println!("rails report: {:#?}", out.report);
}

/// A 300-module graph against the same code as one classic bundle, through
/// `render_page`. `P2_RT_12_COST=modules|bundle` picks one, so peak RSS
/// (VmHWM) is per case; the default runs both.
#[test]
#[ignore = "measurement"]
fn p2_rt_12_cost_page() {
    let n: usize = 300;
    let mut files = HashMap::new();
    let mut bundle = String::from("(function(){\n");
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
        files.insert(format!("/m/g{i}.js"), src.into_bytes());
        bundle.push_str(&body);
    }
    bundle.push_str(&format!(
        "document.getElementById('o').textContent = f{}(1); }})();",
        n - 1
    ));
    files.insert("/m/bundle.js".into(), bundle.into_bytes());
    let modules_page = format!(
        "<!doctype html><p id=o></p><script type=module>import {{ f{0} }} from '/m/g{0}.js'; document.getElementById('o').textContent = f{0}(1);</script>",
        n - 1
    );
    let bundle_page = "<!doctype html><p id=o></p><script src=/m/bundle.js></script>".to_owned();
    let which = std::env::var("P2_RT_12_COST").unwrap_or_default();
    let renderer = Renderer::new(opts().resource_loader(ResourceLoader::from_map(files))).unwrap();
    for (label, page) in [("modules", &modules_page), ("bundle", &bundle_page)] {
        if !which.is_empty() && which != label {
            continue;
        }
        let mut times = Vec::new();
        let mut text = String::new();
        for _ in 0..20 {
            let t = Instant::now();
            let out = renderer.render_page(page).unwrap();
            times.push(t.elapsed());
            text = out.html[out.html.find("<p id=\"o\">").unwrap_or(0)..]
                .chars()
                .take(40)
                .collect();
        }
        times.sort();
        println!(
            "{label}: median {:?}, min {:?}, {text}; VmHWM {}",
            times[times.len() / 2],
            times[0],
            std::fs::read_to_string("/proc/self/status")
                .unwrap_or_default()
                .lines()
                .find(|l| l.starts_with("VmHWM"))
                .unwrap_or("")
        );
    }
}
