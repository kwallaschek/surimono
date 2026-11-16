//! Scripts that scripts insert (HTML §4.12.1.1 "prepare the script element"
//! for an element that isn't parser-inserted, "execute the script element",
//! and §13.2.7 "the end", steps 7 and 8), against pages measured in Chromium
//! 153.0.8010.12 (the pinned Playwright Chromium, every request answered from
//! memory): each case quotes what Chromium logged, and where we differ, why.

mod common;

use std::cell::RefCell;
use std::time::Duration;

use common::{limits, scaled};
use surimono_core::{Limits, SandboxError, SettleOptions};
use surimono_dom::{Document, DomError, DomRuntime, PageReport};

/// What the probe's server answered: `/js/NAME.js` and `/slow/MS/NAME.js`
/// log `NAME` (Chromium waited MS first), `/throw/NAME.js` logs and throws,
/// and anything else has no source (a 404 there).
fn serve(path: &str) -> Option<String> {
    let rest = path
        .strip_prefix("/js/")
        .or_else(|| path.strip_prefix("/throw/"))
        .or_else(|| {
            path.strip_prefix("/slow/")
                .and_then(|p| p.split_once('/').map(|(_, r)| r))
        })?;
    let name = rest.strip_suffix(".js")?;
    Some(match name {
        "cur" => "log('cur',document.currentScript&&document.currentScript.id);Promise.resolve().then(()=>log('cur micro',document.currentScript&&document.currentScript.id));setTimeout(()=>log('cur timer',document.currentScript),0)".to_owned(),
        "chain1" => "log('chain1');var s=document.createElement('script');s.src='/js/chain2.js';document.head.appendChild(s);var i=document.createElement('script');i.textContent=\"log('inline in chain1',document.currentScript.src)\";document.head.appendChild(i);log('chain1 end',document.currentScript.src.slice(-9))".to_owned(),
        "partA" => "var partA=1;log('A')".to_owned(),
        "partB" => "var partB=1;log('B',typeof partA)".to_owned(),
        _ if path.starts_with("/throw/") => format!("log('{name}');throw new Error('boom')"),
        _ => format!("log('{name}')"),
    })
}

/// The probe's page around `body`, with its `log`.
fn wrap(body: &str) -> String {
    format!(
        "<!doctype html><html><head><script>var L=[];function log(){{L.push(Array.prototype.map.call(arguments,String).join(' '))}}</script></head><body>{body}</body></html>"
    )
}

/// Runs `html` as a page with `resolve`; its log, and the report.
fn run(
    html: &str,
    limits: Limits,
    resolve: impl FnMut(&str) -> Option<String>,
) -> (Result<PageReport, DomError>, Vec<String>) {
    let mut rt = DomRuntime::new(limits).unwrap();
    rt.load(Document::parse(html).unwrap());
    let report = rt.run_page(&SettleOptions::default(), resolve);
    let log = match &report {
        Ok(_) => rt.eval::<String>("L.join('\\u0001')").unwrap(),
        Err(_) => String::new(),
    };
    let log = if log.is_empty() {
        Vec::new()
    } else {
        log.split('\u{1}').map(str::to_owned).collect()
    };
    (report, log)
}

const CASES: &[(&str, &str, &[&str])] = &[
    // Chromium 153 logs the same.
    (
        "inline_sync",
        r#"<script id=outer>var s=document.createElement('script');s.id='in';s.textContent="log('ran',document.currentScript&&document.currentScript.id)";document.head.appendChild(s);log('after',document.currentScript.id)</script>"#,
        &["ran in", "after outer"],
    ),
    // The page's lifecycle steps are timers queued before the page's own, so `DOMContentLoaded` comes before a zero-delay timer here; Chromium's order of the two is a race (DESIGN.md, "Events and the page's life").
    // Chromium 153: ["after", "micro", "t0", "DCL", "a", "onload null", "wload"]
    (
        "src_async",
        r#"<script>document.addEventListener('DOMContentLoaded',()=>log('DCL'));window.addEventListener('load',()=>log('wload'));var s=document.createElement('script');s.src='/js/a.js';s.onload=()=>log('onload',document.currentScript);s.onerror=()=>log('onerror');document.head.appendChild(s);log('after');Promise.resolve().then(()=>log('micro'));setTimeout(()=>log('t0'),0)</script>"#,
        &["after", "micro", "DCL", "t0", "a", "onload null", "wload"],
    ),
    // Chromium 153 logs the same.
    (
        "in_order",
        r#"<script>for(const [n,ms] of [['a',60],['b',30],['c',0]]){var s=document.createElement('script');s.async=false;s.src='/slow/'+ms+'/'+n+'.js';document.head.appendChild(s)}</script>"#,
        &["a", "b", "c"],
    ),
    // Departure: async scripts run in the order asked, never by which load finished first (same input, same output). Chromium ran them as they arrived.
    // Chromium 153: ["c", "b", "a"]
    (
        "async_order",
        r#"<script>for(const [n,ms] of [['a',60],['b',30],['c',0]]){var s=document.createElement('script');s.src='/slow/'+ms+'/'+n+'.js';document.head.appendChild(s)}</script>"#,
        &["a", "b", "c"],
    ),
    // Chromium 153 logs the same.
    (
        "async_attr",
        r#"<script id=p>var s=document.createElement('script');log(s.async,s.hasAttribute('async'));s.async=false;log(s.async,s.hasAttribute('async'));s.async=true;log(s.async,s.hasAttribute('async'));s.removeAttribute('async');log(s.async);log(document.getElementById('p').async);var c=document.createElement('script');c.setAttribute('async','');c.removeAttribute('async');log(c.async)</script>"#,
        &[
            "true false",
            "false false",
            "true true",
            "false",
            "false",
            "false",
        ],
    ),
    // Chromium 153 logs the same.
    (
        "started",
        r#"<div id=d></div><script>var s=document.createElement('script');s.textContent="log('run')";document.body.appendChild(s);s.remove();document.body.appendChild(s);document.getElementById('d').appendChild(s);var c=s.cloneNode(true);document.body.appendChild(c);var u=document.createElement('script');u.type='x';u.textContent="log('u')";var uc=u.cloneNode(true);document.body.appendChild(u);uc.removeAttribute('type');document.body.appendChild(uc);log('end')</script>"#,
        &["run", "u", "end"],
    ),
    // Chromium 153 logs the same.
    (
        "detached",
        r#"<script>var d=document.createElement('div');var s=document.createElement('script');s.textContent="log('ran',this===window,document.currentScript===s)";d.appendChild(s);log('in detached');document.body.appendChild(d);log('after connect')</script>"#,
        &["in detached", "ran true true", "after connect"],
    ),
    // Chromium 153 logs the same.
    (
        "template",
        r#"<template id=t><script>log('tmpl')</script></template><script>var t=document.getElementById('t');var c=document.importNode(t.content,true);log('imported');document.body.appendChild(c);log('appended clone');document.body.appendChild(t.content);log('appended content')</script>"#,
        &[
            "imported",
            "tmpl",
            "appended clone",
            "tmpl",
            "appended content",
        ],
    ),
    // Chromium 153 logs the same.
    (
        "template2",
        r#"<script>var t=document.createElement('template');var s=document.createElement('script');s.textContent="log('in tmpl')";t.content.appendChild(s);document.body.appendChild(t);log('tmpl appended');document.body.appendChild(t.content);log('content appended')</script>"#,
        &["tmpl appended", "in tmpl", "content appended"],
    ),
    // Chromium 153 logs the same.
    (
        "settext",
        r#"<script>var s=document.createElement('script');document.body.appendChild(s);log('empty in');s.text="log('text set')";log('after text');var r=document.createElement('script');document.body.appendChild(r);r.appendChild(document.createTextNode("log('child')"));log('after child');r.text="log('again')";log('end')</script>"#,
        &[
            "empty in",
            "text set",
            "after text",
            "child",
            "after child",
            "end",
        ],
    ),
    // Chromium 153 logs the same. DOM's "replace data" runs the children
    // changed steps, which would run it; we follow Chromium and WPT (DESIGN.md).
    (
        "setdata",
        r#"<script>var s=document.createElement('script');var t=document.createTextNode('');s.appendChild(t);document.body.appendChild(s);log('in');t.data="log('data')";log('after data');t.appendData(";log('more')");log('after append')</script>"#,
        &["in", "after data", "after append"],
    ),
    // Chromium 153 logs the same.
    (
        "setsrc",
        r#"<script>var s=document.createElement('script');document.body.appendChild(s);s.onload=()=>log('load');s.src='/js/x.js';log('after src');var r=document.createElement('script');r.textContent="log('inline r')";document.body.appendChild(r);r.onload=()=>log('r load');r.src='/js/y.js';log('end')</script>"#,
        &["after src", "inline r", "end", "x", "load"],
    ),
    // Chromium 153 logs the same.
    (
        "setsrc2",
        r#"<script>var s=document.createElement('script');s.type='text/plain';s.textContent="log('plain')";document.body.appendChild(s);s.type='';log('type changed');s.src='/js/z.js';log('src set');s.removeAttribute('src');s.setAttribute('src','/js/w.js');log('end')</script>"#,
        &["type changed", "src set", "end", "z"],
    ),
    // Chromium 153 logs the same, as WPT script-does-not-run-on-child-removal
    // asks, though DOM's "remove" runs the children changed steps.
    (
        "removechild",
        r#"<script>var s=document.createElement('script');s.type='bogus';s.append("log('one');",document.createElement('b'));document.body.appendChild(s);s.type='';s.lastChild.remove();log('after remove')</script>"#,
        &["after remove"],
    ),
    // Chromium 153 logs the same.
    (
        "types",
        r#"<script>for(const [k,v] of [['type','text/plain'],['type',' text/javascript '],['type','TEXT/JAVASCRIPT'],['type','application/ecmascript'],['type','text/javascript;charset=utf-8'],['language','javascript1.5'],['language','vbscript'],['language',''],['type','']]){var s=document.createElement('script');s.setAttribute(k,v);s.textContent="log('ran "+k+"="+v+"')";document.body.appendChild(s)}log('end')</script>"#,
        &[
            "ran type= text/javascript ",
            "ran type=TEXT/JAVASCRIPT",
            "ran type=application/ecmascript",
            "ran language=javascript1.5",
            "ran language=",
            "ran type=",
            "end",
        ],
    ),
    // Departure: module scripts don't run on main (P2-RT-12), and do nothing: no `error`, as for the parser's.
    // Chromium 153: ["after", "module ran"]
    (
        "module",
        r#"<script>var s=document.createElement('script');s.type='module';s.textContent="log('module ran')";s.onerror=()=>log('module error');document.body.appendChild(s);log('after')</script>"#,
        &["after"],
    ),
    // Departure: `nomodule` is ignored, as for the parser's scripts, since modules don't run here (P2-RT-12's step 10 reverses it).
    // Chromium 153: ["after"]
    (
        "nomodule",
        r#"<script>var s=document.createElement('script');s.noModule=true;s.textContent="log('nomodule ran')";document.body.appendChild(s);var r=document.createElement('script');r.setAttribute('nomodule','');r.src='/js/nm.js';r.onload=()=>log('nm load');r.onerror=()=>log('nm error');document.body.appendChild(r);log('after')</script>"#,
        &["nomodule ran", "after", "nm", "nm load"],
    ),
    // Chromium 153 logs the same.
    (
        "cur_async",
        r#"<script>window.cs=[];var s=document.createElement('script');s.id='ext';s.src='/js/cur.js';document.head.appendChild(s)</script>"#,
        &["cur ext", "cur micro ext", "cur timer null"],
    ),
    // Same order; QuickJS's message has no `Uncaught ` prefix.
    // Chromium 153: ["sync end", "error  Event false false", "error /404/a.js Event false false", "b", "window error Uncaught Error: boom", "load /throw/b.js"]
    (
        "loaderr",
        r#"<script>window.onerror=(m)=>{log('window error',String(m).slice(0,20))};function mk(src){var s=document.createElement('script');s.src=src;s.onload=()=>log('load',src);s.onerror=(e)=>log('error',src,e.constructor.name,e.bubbles,e.cancelable);document.head.appendChild(s)}mk('/404/a.js');mk('/throw/b.js');mk('');log('sync end')</script>"#,
        &[
            "sync end",
            "error  Event false false",
            "error /404/a.js Event false false",
            "b",
            "window error Error: boom",
            "load /throw/b.js",
        ],
    ),
    // Chromium 153 logs the same.
    (
        "loaderr2",
        r#"<script>var s=document.createElement('script');s.src='/404/q.js';s.addEventListener('error',()=>log('error at el'));document.addEventListener('error',(e)=>log('doc capture',e.target===s),true);window.addEventListener('error',(e)=>log('win',e.target===s),true);document.head.appendChild(s)</script>"#,
        &["win true", "doc capture true", "error at el"],
    ),
    // Same order; the message is QuickJS's.
    // Chromium 153: ["x", "onerror Uncaught TypeError: Cannot rea", "after"]
    (
        "inline_throw",
        r#"<script>window.onerror=(m)=>{log('onerror',String(m).slice(0,30))};var s=document.createElement('script');s.textContent="log('x');null.f()";s.onerror=()=>log('el error');s.onload=()=>log('el load');document.body.appendChild(s);log('after')</script>"#,
        &["x", "onerror TypeError: cannot read propert", "after"],
    ),
    // Chromium 153 logs the same.
    (
        "innerhtml",
        r#"<div id=d></div><script>var d=document.getElementById('d');d.innerHTML="<script>log('innerHTML')<\/script>";log('a');d.insertAdjacentHTML('beforeend',"<script>log('adjacent')<\/script>");log('b');var e=document.createElement('p');d.appendChild(e);e.outerHTML="<script>log('outer')<\/script>";log('c');var moved=d.querySelector('script');document.body.appendChild(moved);log('d')</script>"#,
        &["a", "b", "c", "d"],
    ),
    // Chromium 153 logs the same.
    (
        "innerhtml_clone",
        r#"<div id=d></div><script>var d=document.getElementById('d');d.innerHTML="<script>log('ih')<\/script>";var c=d.firstChild.cloneNode(true);document.body.appendChild(c);var n=document.createElement('script');n.textContent=d.firstChild.textContent;document.body.appendChild(n);log('end')</script>"#,
        &["ih", "end"],
    ),
    // Chromium 153 logs the same.
    (
        "ctxfrag",
        r#"<script>var f=document.createRange().createContextualFragment("<script>log('ctx inline')<\/script><script src='/js/ctx.js'><\/script>");log('made');document.body.appendChild(f);log('appended')</script>"#,
        &["made", "ctx inline", "appended", "ctx"],
    ),
    // Chromium 153 logs the same.
    (
        "domparser",
        r#"<script>var d=new DOMParser().parseFromString("<script>log('parsed')<\/script>",'text/html');var s=d.querySelector('script');document.body.appendChild(s);log('adopted+appended');var e=document.createElement('script');e.textContent="log('fresh')";d.body.appendChild(e);log('into parsed doc');document.body.appendChild(e);log('moved to page')</script>"#,
        &["adopted+appended", "into parsed doc", "moved to page"],
    ),
    // Chromium 153 logs the same.
    (
        "otherdoc",
        r#"<script>var d=document.implementation.createHTMLDocument('');var e=d.createElement('script');e.textContent="log('made in other doc')";d.body.appendChild(e);log('in other');document.body.appendChild(e);log('moved in')</script>"#,
        &["in other", "moved in"],
    ),
    // Chromium 153 logs the same.
    (
        "moved_before_exec",
        r#"<script>var s=document.createElement('script');s.src='/js/mv.js';s.onload=()=>log('mv load');s.onerror=()=>log('mv error');document.body.appendChild(s);var d=document.implementation.createHTMLDocument('');d.body.appendChild(s);log('moved')</script>"#,
        &["moved"],
    ),
    // Chromium 153 logs the same.
    (
        "removed_before_exec",
        r#"<script>var s=document.createElement('script');s.src='/js/rm.js';s.onload=()=>log('rm load',s.isConnected);document.body.appendChild(s);s.remove();s.src='/js/other.js';log('removed')</script>"#,
        &["removed", "rm", "rm load false"],
    ),
    // Frames run no scripts here (DESIGN.md, "Frames").
    // Chromium 153: ["appended", "in frame", "frame load"]
    (
        "iframe",
        r#"<script>var f=document.createElement('iframe');f.srcdoc="<script>parent.log('in frame')<\/script>";f.onload=()=>log('frame load');document.body.appendChild(f);log('appended')</script>"#,
        &["appended", "frame load"],
    ),
    // Chromium 153 logs the same.
    (
        "selfloop",
        r#"<script id=me>window.k=(window.k||0)+1;for(var i=0;i<3;i++)document.body.appendChild(document.currentScript);log('k',k)</script>"#,
        &["k 1"],
    ),
    // Chromium 153 logs the same.
    (
        "children_order",
        r#"<script id=outer></script><script>var o=document.getElementById('outer');var inner=document.createElement('script');inner.textContent="log('inner')";o.append(new Text("log(1);"),inner,new Text("log(2);"));log('done')</script>"#,
        &["1", "2", "inner", "done"],
    ),
    // Chromium 153 logs the same.
    (
        "removes_other",
        r#"<script>var s1=document.createElement('script');s1.textContent="log('s1');document.querySelector('#s2').remove()";var s2=document.createElement('script');s2.id='s2';s2.textContent="log('s2')";document.body.append(s1,s2);log('end')</script>"#,
        &["s1", "end"],
    ),
    // Chromium 153 logs the same.
    (
        "append_two",
        r#"<script>var a=document.createElement('script');a.textContent="log('a',document.querySelectorAll('script').length)";var b=document.createElement('script');b.textContent="log('b')";var f=document.createDocumentFragment();f.append(a,b);document.body.appendChild(f);log('end')</script>"#,
        &["a 4", "b", "end"],
    ),
    // Chromium 153 logs the same.
    (
        "parser_empty",
        r#"<script id=e></script><script>var e=document.getElementById('e');e.textContent="log('empty parser script ran')";log('after');log(e.async)</script>"#,
        &["empty parser script ran", "after", "true"],
    ),
    // Chromium 153 logs the same.
    (
        "parser_badtype",
        r#"<script id=e type=bogus>log('bad')</script><script>var e=document.getElementById('e');e.type='';e.appendChild(document.createTextNode(';log(2)'));log('after')</script>"#,
        &["bad", "2", "after"],
    ),
    // Chromium 153 logs the same.
    (
        "delays_load",
        r#"<script>window.addEventListener('load',()=>log('wload'));document.addEventListener('DOMContentLoaded',()=>log('DCL'));var s=document.createElement('script');s.src='/slow/100/late.js';document.head.appendChild(s)</script>"#,
        &["DCL", "late", "wload"],
    ),
    // Chromium 153 logs the same.
    (
        "after_load",
        r#"<script>window.addEventListener('load',()=>{var s=document.createElement('script');s.src='/js/afterload.js';s.onload=()=>log('al load');document.head.appendChild(s);setTimeout(()=>log('t0'),0);log('in load')})</script>"#,
        &["in load", "t0", "afterload", "al load"],
    ),
    // Chromium 153 logs the same.
    (
        "chain",
        r#"<script>var s=document.createElement('script');s.src='/js/chain1.js';document.head.appendChild(s)</script>"#,
        &[
            "chain1",
            "inline in chain1 ",
            "chain1 end chain1.js",
            "chain2",
        ],
    ),
    // Chromium 153 logs the same.
    (
        "loader",
        r#"<script>function load(src,cb){var s=document.createElement('script');s.src=src;s.async=false;s.onload=cb;document.head.appendChild(s)}var left=2;function done(){if(--left===0)log('ready',typeof partA,typeof partB)}load('/js/partA.js',done);load('/js/partB.js',done)</script>"#,
        &["A", "B number", "ready number number"],
    ),
    // `<svg><script>` doesn't run here, inserted or parsed (DESIGN.md gap).
    // Chromium 153: ["svg script", "after"]
    (
        "svgscript",
        r#"<svg id=v></svg><script>var s=document.createElementNS('http://www.w3.org/2000/svg','script');s.textContent="log('svg script')";document.getElementById('v').appendChild(s);log('after')</script>"#,
        &["after"],
    ),
    // Chromium 153 logs the same.
    (
        "xhtmlns",
        r#"<script>var s=document.createElementNS('http://www.w3.org/1999/xhtml','script');s.textContent="log('ns script')";document.body.appendChild(s);log('after')</script>"#,
        &["ns script", "after"],
    ),
    // Chromium 153 logs the same.
    (
        "tmpl_parser",
        r#"<template id=t><script>log('tmpl direct')</script></template><script>document.body.appendChild(document.getElementById('t').content);log('end')</script>"#,
        &["tmpl direct", "end"],
    ),
    // Departure: as `async_order`.
    // Chromium 153: ["b", "d", "c", "a"]
    (
        "mixed",
        r#"<script>for(const [n,ms,a] of [['a',60,1],['b',0,0],['c',30,1],['d',0,0]]){var s=document.createElement('script');s.async=!!a;s.src='/slow/'+ms+'/'+n+'.js';document.head.appendChild(s)}</script>"#,
        &["a", "b", "c", "d"],
    ),
    // Chromium 153 logs the same.
    (
        "inorder_err",
        r#"<script>for(const n of ['/js/p.js','/404/q.js','/js/r.js']){var s=document.createElement('script');s.async=false;s.src=n;s.onerror=()=>log('err');s.onload=()=>log('ld');document.head.appendChild(s)}</script>"#,
        &["p", "ld", "err", "r", "ld"],
    ),
    // Chromium 153 logs the same.
    (
        "text_replace",
        r#"<script>var s=document.createElement('script');s.append(new Text(''));document.body.appendChild(s);s.firstChild.replaceWith(new Text("log('replaced')"));log('end')</script>"#,
        &["replaced", "end"],
    ),
    // Chromium 153 logs the same.
    (
        "dcl_insert",
        r#"<script>document.addEventListener('DOMContentLoaded',()=>{var s=document.createElement('script');s.textContent="log('in dcl',document.readyState)";document.body.appendChild(s);var e=document.createElement('script');e.src='/js/dclext.js';document.body.appendChild(e)});window.onload=()=>log('wload')</script>"#,
        &["in dcl interactive", "dclext", "wload"],
    ),
    // A fetch takes no virtual time, so the script runs before a later timer; in Chromium the two race (this run of the probe saw `t5` first, an earlier one `tm` first).
    // Chromium 153: ["mic", "t5", "tm", "tm load"]
    (
        "timer_insert",
        r#"<script>setTimeout(()=>{var e=document.createElement('script');e.src='/js/tm.js';e.onload=()=>log('tm load');document.body.appendChild(e);setTimeout(()=>log('t5'),5);Promise.resolve().then(()=>log('mic'))},10)</script>"#,
        &["mic", "tm", "tm load", "t5"],
    ),
    // Chromium 153 logs the same.
    (
        "event_attr",
        r#"<script>var s=document.createElement('script');s.setAttribute('event','onload');s.setAttribute('for','window');s.textContent="log('for window')";document.body.appendChild(s);var r=document.createElement('script');r.setAttribute('event','onclick');r.setAttribute('for','window');r.textContent="log('for click')";document.body.appendChild(r);log('end')</script>"#,
        &["for window", "end"],
    ),
];

#[test]
fn inserted_scripts_against_chromium() {
    let mut wrong = Vec::new();
    for (name, body, expected) in CASES {
        let (report, log) = run(&wrap(body), limits(), serve);
        if let Err(e) = report {
            wrong.push(format!("{name}: failed with {e}"));
            continue;
        }
        if !log.iter().map(String::as_str).eq(expected.iter().copied()) {
            wrong.push(format!("{name}: {log:?}, expected {expected:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// An unclosed `<script>` at the end of a fragment's input was marked
/// started by the parser, and stays so in a contextual fragment (HTML
/// §13.2.6.4.8; WPT domparsing/tentative/unclosed-script-already-started); a
/// closed one runs.
#[test]
fn an_unclosed_script_in_a_contextual_fragment_never_runs() {
    let (report, log) = run(
        &wrap(
            "<div id=c></div><script>var c = document.getElementById('c'), r = document.createRange();
             r.selectNodeContents(c);
             c.appendChild(r.createContextualFragment('<script>log(1)<\\/script><script id=u>log(2)'));
             var u = document.getElementById('u'); u.appendChild(new Text('log(3)'));
             c.appendChild(u.cloneNode(true)); log('end')</script>",
        ),
        limits(),
        |_| None,
    );
    assert_eq!(report.map(|r| r.uncaught), Ok(0));
    assert_eq!(log, ["1", "end"]);
}

/// A `<script>` still open at the end of the page's input: the parser marks
/// it started (HTML §13.2.6.4.8), so it never runs. Chromium 153 doesn't run
/// it from the parse either (`[]`), but runs it once a script appends a child
/// to it (`["unclosed ran","appended","load 3"]`); the spec wins.
#[test]
fn an_unclosed_script_at_the_end_of_the_page_never_runs() {
    let head = "<!doctype html><html><head><script>var L=[];function log(){L.push(Array.prototype.map.call(arguments,String).join(' '))}</script>";
    let (report, log) = run(
        &format!("{head}</head><body><p>x</p><script>log('unclosed ran')"),
        limits(),
        |_| None,
    );
    assert_eq!(report.map(|r| r.uncaught), Ok(0));
    assert!(log.is_empty(), "{log:?}");
    let (report, log) = run(
        &format!(
            "{head}<script>addEventListener('load', () => {{ var s = document.scripts[document.scripts.length - 1];
             s.appendChild(new Text(';log(\"appended\")')); document.body.appendChild(s.cloneNode(true));
             log('load ' + document.scripts.length) }})</script><script>log('unclosed ran')"
        ),
        limits(),
        |_| None,
    );
    assert_eq!(report.map(|r| r.uncaught), Ok(0));
    assert_eq!(log, ["load 4"]);
}

/// The loader is asked in the order the page asked, once nothing else is due
/// at that time, and the scripts run in that order: how long it took to
/// answer changes nothing.
#[test]
fn the_order_never_depends_on_how_fast_the_loader_answers() {
    let page = wrap(
        "<script>for (const n of ['a','b','c','d']) { var s = document.createElement('script');
           s.async = n === 'b'; s.src = '/js/' + n + '.js'; s.onload = () => log('load ' + n);
           document.head.appendChild(s) }
           setTimeout(() => log('t0'), 0); setTimeout(() => log('t1'), 1)</script>",
    );
    let mut runs = Vec::new();
    for delays in [[0, 0, 0, 0], [30, 0, 20, 5], [0, 25, 0, 40]] {
        let asked = RefCell::new(Vec::new());
        let (report, log) = run(&page, limits(), |url| {
            let i = asked.borrow().len();
            asked.borrow_mut().push(url.to_owned());
            std::thread::sleep(Duration::from_millis(delays[i % 4]));
            serve(url)
        });
        assert_eq!(report.map(|r| r.uncaught), Ok(0));
        assert_eq!(
            asked.into_inner(),
            ["/js/a.js", "/js/b.js", "/js/c.js", "/js/d.js"]
        );
        runs.push(log);
    }
    assert_eq!(
        runs[0],
        [
            "t0", "a", "load a", "b", "load b", "c", "load c", "d", "load d", "t1"
        ]
    );
    assert!(runs.iter().all(|r| *r == runs[0]), "{runs:?}");
}

/// A fetch the resolver has no source for: `error` at the element, the URL
/// in the report, and the page goes on.
#[test]
fn a_failed_fetch_is_an_error_event_and_a_missing_script() {
    let (report, log) = run(
        &wrap(
            "<script>var s = document.createElement('script'); s.src = 'nope.js';
             s.onerror = (e) => log('error', e.isTrusted); document.head.appendChild(s);
             addEventListener('load', () => log('window load'))</script>",
        ),
        limits(),
        |_| None,
    );
    let report = report.unwrap();
    assert_eq!(log, ["error true", "window load"]);
    assert_eq!(report.missing_scripts, ["nope.js"]);
    assert_eq!(report.uncaught, 0);
}

/// Without a page's resolver (`eval` and `run_until_settled`), a fetch
/// fails as if the resolver had nothing.
#[test]
fn outside_a_page_a_fetch_fails() {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse("<!doctype html><title>x</title>").unwrap());
    let got = rt
        .eval::<String>(
            "globalThis.L = []; var s = document.createElement('script'); s.src = 'x.js';
             s.onerror = () => L.push('error'); document.head.appendChild(s);
             var i = document.createElement('script'); i.text = \"L.push('inline')\";
             document.head.appendChild(i); L.join()",
        )
        .unwrap();
    assert_eq!(got, "inline");
    rt.run_until_settled(&SettleOptions::default()).unwrap();
    assert_eq!(rt.eval::<String>("L.join()").unwrap(), "inline,error");
}

/// Chromium 153 stops nesting at 44 with a `RangeError` (probe `depth`,
/// `deep`). Here a script a script inserted may run inside 31 others; one
/// more ends the page with the stack limit's typed error, at the same depth
/// on every build. The stack limit is raised so that the cap is what ends
/// it (a debug build with the default 512 KiB reaches it at about 25): the
/// default's end is the same error, 100,000 deep, and the host's stack is
/// never at risk, on a thread with 1 MiB (as much as Ruby's) beyond the
/// stack limit.
#[test]
fn nesting_ends_in_a_typed_limit() {
    let nest = |depth: u32| {
        wrap(&format!(
            "<script>window.n = 0; function go() {{ var s = document.createElement('script');
             s.textContent = 'n++; if (n < {depth}) go()'; document.body.appendChild(s) }}
             try {{ go() }} catch (e) {{ log('caught', e.name) }} log(n)</script>"
        ))
    };
    for limits in [limits().with_stack_bytes(8 * 1024 * 1024), limits()] {
        let stack = limits.stack_bytes + 1024 * 1024;
        let raised = limits.stack_bytes > default_stack_bytes();
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn(move || {
                if raised {
                    let (report, log) = run(&nest(32), limits, |_| None);
                    assert_eq!(report.map(|r| r.uncaught), Ok(0));
                    assert_eq!(log, ["32"]);
                }
                let depths: &[u32] = if raised { &[33, 100_000] } else { &[100_000] };
                for &depth in depths {
                    let (report, _) = run(&nest(depth), limits, |_| None);
                    assert_eq!(
                        report.err(),
                        Some(DomError::Sandbox(SandboxError::StackOverflow)),
                        "{depth}"
                    );
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

fn default_stack_bytes() -> usize {
    Limits::default().stack_bytes
}

/// The cap, not the stack limit, ends nesting with the default limits in a
/// release build: 32 deep runs. Release only, since a debug build's frames
/// reach the stack limit near 25 (see `nesting_ends_in_a_typed_limit`); the
/// soak job runs the same check (`tests/soak.rs`).
#[cfg(not(debug_assertions))]
#[test]
fn nesting_32_deep_fits_the_default_stack() {
    let html = wrap(
        "<script>window.n = 0; function go() { var s = document.createElement('script');
         s.textContent = 'n++; if (n < 32) go()'; document.body.appendChild(s) }
         go(); log(n)</script>",
    );
    let limits = Limits::default();
    let stack = limits.stack_bytes + 1024 * 1024;
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || {
            let (report, log) = run(&html, limits, |_| None);
            assert_eq!(report.map(|r| r.uncaught), Ok(0));
            assert_eq!(log, ["32"]);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// A loop of a million inline scripts, and one of a million to fetch, end in
/// a typed limit within the time limit's bounds: the time limit (each runs
/// as a script), or the document's handle limit (each is a node a script
/// holds).
#[test]
fn a_million_inserted_scripts_end_in_a_typed_limit() {
    for body in [
        "for (var i = 0; i < 1e6; i++) { var s = document.createElement('script');
           s.textContent = 'c++'; document.body.appendChild(s); s.remove() }",
        "for (var i = 0; i < 1e6; i++) { var s = document.createElement('script');
           s.src = 'x' + i + '.js'; document.body.appendChild(s) }",
    ] {
        let limits = limits().with_time(scaled(Duration::from_millis(500)));
        let started = std::time::Instant::now();
        let (report, _) = run(
            &wrap(&format!("<script>var c = 0; {body}</script>")),
            limits,
            |_| None,
        );
        let e = report.unwrap_err();
        assert!(
            matches!(
                e,
                DomError::Sandbox(SandboxError::TimeLimit) | DomError::HandleLimit { .. }
            ),
            "{e}"
        );
        assert!(
            started.elapsed() < scaled(Duration::from_secs(5)),
            "{:?}",
            started.elapsed()
        );
    }
}

/// Past 10,000 fetches in a render, the rest fail without reaching the
/// resolver, and their `error` still fires, in its place in the order asked:
/// after the earlier fetches' `load` and `error`, not ahead of them.
#[test]
fn fetches_past_the_cap_fail_without_the_resolver_in_order() {
    let calls = RefCell::new(0usize);
    let (report, log) = run(
        &wrap(
            "<script>var errors = 0, order = []; for (let i = 0; i < 10005; i++) {
               var s = document.createElement('script'); s.src = i === 9999 ? 'ok.js' : 'f.js';
               s.onerror = () => { errors++; order.push(i) }; s.onload = () => order.push(i);
               document.head.appendChild(s) }
             addEventListener('load', () => log(errors, order.length, order.every((v, k) => v === k)))</script>",
        ),
        limits().with_time(scaled(Duration::from_secs(30))),
        |url| {
            *calls.borrow_mut() += 1;
            (url == "ok.js").then(String::new)
        },
    );
    let report = report.unwrap();
    assert_eq!(*calls.borrow(), 10_000);
    assert_eq!(log, ["10004 10005 true"]);
    assert!(report.missing_scripts.len() >= 9_999);
}

/// A pooled runtime keeps nothing of a render's inserted scripts: the next
/// page of the same runtime sees none of them, and the heap after a reset is
/// the same after 50 renders as after one.
#[test]
fn a_render_leaves_nothing_in_a_pooled_runtime() {
    let page = wrap(
        "<script>for (var i = 0; i < 20; i++) { var s = document.createElement('script');
           s.src = '/js/p' + (i % 3) + '.js'; document.head.appendChild(s);
           var t = document.createElement('script'); t.text = 'log(1)'; document.head.appendChild(t) }
         var late = document.createElement('script'); late.src = '/js/late.js';
         addEventListener('load', () => document.head.appendChild(late))</script>",
    );
    let mut rt = DomRuntime::new(limits()).unwrap();
    let mut heap = Vec::new();
    for _ in 0..50 {
        rt.load(Document::parse(&page).unwrap());
        let report = rt.run_page(&SettleOptions::default(), serve).unwrap();
        assert_eq!(report.uncaught, 0);
        assert_eq!(rt.eval::<u32>("L.length").unwrap(), 20 + 20 + 1);
        drop(rt.take());
        rt.reset().unwrap();
        heap.push(rt.sandbox().heap_bytes());
    }
    assert_eq!(heap[1], heap[49], "{heap:?}");
}
