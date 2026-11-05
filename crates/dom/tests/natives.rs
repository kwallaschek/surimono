//! A native can't be interrupted, so a script must not be able to make one run
//! long: each case here is the worst input a page within the limits can give
//! one, under a short time limit. It either finishes or ends in `TimeLimit`,
//! close to the limit (reviews of #94 and #96, where these ran for 100 s).

mod common;

use std::time::{Duration, Instant};

use common::{limits, scaled};
use surimono_core::SandboxError;
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

const LIMIT: Duration = Duration::from_millis(300);

/// Runs `script` on `page` under [`LIMIT`], and checks how it ended.
/// `Some(result)` if it finished.
fn bounded(page: &str, script: &str) -> Option<String> {
    let limit = scaled(LIMIT);
    let mut rt = DomRuntime::new(limits().with_time(limit)).unwrap();
    let doc = Document::parse_with_limits(
        page,
        DomLimits {
            max_nodes: 200_000,
            ..DomLimits::default()
        },
    )
    .unwrap();
    rt.load(doc);
    // Installs the bindings, which isn't what is measured.
    let _: () = rt.eval("0").unwrap();
    let started = Instant::now();
    let r = rt.eval::<String>(script);
    let took = started.elapsed();
    // Well above the limit, for a loaded machine; far below an unbounded loop.
    assert!(took < limit + scaled(Duration::from_secs(2)), "{took:?}");
    match r {
        Ok(s) => Some(s),
        Err(DomError::Sandbox(SandboxError::TimeLimit)) => None,
        Err(e) => panic!("{e:?}"),
    }
}

fn unique_tokens(n: usize) -> String {
    (0..n).map(|i| format!("t{i} ")).collect()
}

#[test]
fn class_list_with_many_tokens() {
    let page = format!(
        r#"<!doctype html><p id="p" class="{}">"#,
        unique_tokens(100_000)
    );
    let r = bounded(
        &page,
        "String(document.getElementById('p').classList.length)",
    );
    assert!(r.is_none_or(|n| n == "100000"));
}

#[test]
fn class_list_add_and_remove_with_many_arguments() {
    let page = format!(
        r#"<!doctype html><p id="p" class="{}">"#,
        unique_tokens(50_000)
    );
    let script = "const l = document.getElementById('p').classList;
        const more = []; for (let i = 0; i < 60000; i++) more.push('n' + i);
        l.add(...more); l.remove(...more); String(l.length)";
    assert!(bounded(&page, script).is_none_or(|n| n == "50000"));
}

#[test]
fn collection_with_many_unique_names() {
    let page = format!(
        "<!doctype html>{}",
        (0..49_000)
            .map(|i| format!(r#"<b id="i{i}"></b>"#))
            .collect::<String>()
    );
    let script = "String(Object.keys(document.getElementsByTagName('b')).length)";
    assert!(bounded(&page, script).is_none_or(|n| n == "98000"));
}

#[test]
fn class_lookup_with_many_names_on_many_classes() {
    let page = format!(r#"<!doctype html><p class="{}a">"#, "x ".repeat(100_000));
    let script = "String(document.getElementsByClassName('a '.repeat(100000)).length)";
    assert!(bounded(&page, script).is_none_or(|n| n == "1"));
}

#[test]
fn class_lookup_with_many_unique_names() {
    let page = format!(r#"<!doctype html><p class="{}">"#, unique_tokens(100_000));
    let names: String = unique_tokens(100_000);
    let script = format!("String(document.getElementsByClassName('{names}').length)");
    assert!(bounded(&page, &script).is_none_or(|n| n == "1"));
}

#[test]
fn a_long_comma_list_on_many_elements() {
    let page = format!("<!doctype html>{}", "<b></b>".repeat(20_000));
    let script = "String(document.querySelectorAll(Array(60000).fill('x').join()).length)";
    assert_eq!(bounded(&page, script), None, "20,000 × 60,000 can't finish");
}

#[test]
fn sibling_combinators_across_many_comments() {
    let page = format!(
        "<!doctype html><body><b></b>{}<b></b>",
        "<!--c-->".repeat(90_000)
    );
    let script = "String(document.body.querySelectorAll('b + b').length)";
    assert!(bounded(&page, script).is_none_or(|n| n == "1"));
}

#[test]
fn sibling_combinators_in_a_wide_tree_stay_linear_enough_to_finish() {
    // One sibling step per element, not one scan of the parent per element.
    let page = format!("<!doctype html><body>{}", "<b></b>".repeat(30_000));
    let script = "String(document.body.querySelectorAll('b + b').length)";
    assert_eq!(bounded(&page, script).as_deref(), Some("29999"));
}

#[test]
fn repeated_tests_of_one_elements_children_and_attributes() {
    let long = "x ".repeat(100_000);
    let page = format!(
        r#"<!doctype html><body><i class="{long}" title="{long}">{}</i>"#,
        "<!--c-->".repeat(50_000)
    );
    for selector in ["i:empty", "i.a", "i[title~=a]"] {
        let script = format!("document.querySelectorAll('{selector},'.repeat(10000) + 'x'); ''");
        bounded(&page, &script);
    }
}

/// A runtime on `page` where `v` holds the value of the expression `value`,
/// which isn't what is measured: on a loaded machine, building a long string
/// can take all of a short limit.
fn runtime_with(page: &str, value: &str) -> DomRuntime {
    let mut rt = DomRuntime::new(limits().with_time(scaled(Duration::from_secs(10)))).unwrap();
    rt.load(Document::parse(page).unwrap());
    let _: () = rt.eval(&format!("globalThis.v = {value}; 0")).unwrap();
    rt
}

/// For what must end in `DomError::StyleLimit`, before stylo: the caps don't
/// wait for the deadline, so `write`, a script using `v` (see [`runtime_with`]),
/// is over at once.
fn style_refused(page: &str, value: &str, write: &str) {
    let mut rt = runtime_with(page, value);
    let started = Instant::now();
    let r = rt.eval::<()>(write);
    let took = started.elapsed();
    assert!(
        matches!(r, Err(DomError::StyleLimit { .. })),
        "{write}, {value}: {r:?}"
    );
    assert!(
        took < scaled(Duration::from_secs(2)),
        "{write}, {value}: {took:?}"
    );
}

const STYLED: &str = r#"<!doctype html><body><p id="p"></p>"#;

#[test]
fn a_huge_font_family_through_set_property() {
    // 8 MiB of tokens took 1.12 s in a release build, 2.78 s in a debug one,
    // under a 300 ms limit (review of #97).
    style_refused(
        STYLED,
        "'a,'.repeat(4e6) + 'a'",
        "document.getElementById('p').style.setProperty('font-family', v)",
    );
}

#[test]
fn a_huge_style_through_set_attribute() {
    let p = "document.getElementById('p')";
    // 20 MB took 1.8 s in a release build under 100 ms (review of #97).
    for value in [
        "'font-family:' + 'a,'.repeat(1e7) + 'a'",
        "'font-family:' + 'a,'.repeat(4e6 - 8) + 'a'",
        "'x'.repeat(2e7)",
    ] {
        style_refused(STYLED, value, &format!("{p}.setAttribute('style', v)"));
        style_refused(
            STYLED,
            value,
            &format!("{p}.setAttributeNS('urn:x', 'style', v)"),
        );
    }
    // Just over the cap, or feeding the parser itself runs into the limit.
    let value = "'font-family:' + 'a,'.repeat(40000) + 'a'";
    for html in [
        format!("'<b style=\"' + {value} + '\">'"),
        format!("'<template><b style=\"' + {value} + '\"></template>'"),
    ] {
        style_refused(STYLED, &html, &format!("{p}.innerHTML = v"));
    }
}

#[test]
fn deep_brackets_in_a_style_attribute() {
    let p = "document.getElementById('p')";
    let value = "'background: ' + 'a('.repeat(33)";
    style_refused(STYLED, value, &format!("{p}.setAttribute('style', v)"));
    style_refused(
        STYLED,
        &format!("'<b style=\"' + {value} + '\">'"),
        &format!("{p}.innerHTML = v"),
    );
    let value = "background: ".to_owned() + &"a(".repeat(32);
    let page = format!(r#"<!doctype html><p style="{value}">"#);
    assert!(Document::parse(&page).is_ok());
}

/// A page is refused like a script: stylo runs on its attributes at the parse.
#[test]
fn a_page_with_a_style_attribute_over_the_limits() {
    let value = "background: ".to_owned() + &"a(".repeat(33);
    let long = format!("font-family: {}a", "a,".repeat(1 << 16));
    for page in [
        format!(r#"<!doctype html><p style="{value}">"#),
        format!(r#"<!doctype html><p style="{long}">"#),
        format!(r#"<!doctype html><body><p><body style="{long}">"#),
        format!(r#"<!doctype html><html><p><html style="{long}">"#),
        format!(r#"<!doctype html><svg style="{long}">"#),
    ] {
        assert!(matches!(
            Document::parse(&page),
            Err(DomError::StyleLimit { .. })
        ));
    }
}

#[test]
fn a_style_attribute_at_the_cap_stays_near_the_limit() {
    let script = "const p = document.getElementById('p');
        p.setAttribute('style', 'font-family:' + 'a,'.repeat(32000) + 'a');
        p.setAttribute('style', 'font-family:' + 'a,'.repeat(32000) + 'b');
        String(p.getAttribute('style').length)";
    assert!(bounded(STYLED, script).is_none_or(|n| n == "64013"));
}

/// The ways a script can set a `style` attribute to the value of the
/// expression `value`. All are refused; `cssText` goes through its own parse.
fn refuse_on_every_write(value: &str) {
    let p = "document.getElementById('p')";
    style_refused(STYLED, value, &format!("{p}.setAttribute('style', v)"));
    style_refused(
        STYLED,
        &format!("'<b style=\"' + {value} + '\">'"),
        &format!("{p}.innerHTML = v"),
    );
    style_refused(
        STYLED,
        value,
        &format!("{p}.setAttribute('style', ''); {p}.attributes.getNamedItem('style').value = v"),
    );
    // What it keeps of the text is written back. It keeps one `all`, and what
    // stylo refuses goes, so this may be accepted.
    let mut rt = runtime_with(STYLED, value);
    let script = format!("try {{ {p}.style.cssText = v }} catch (e) {{}} 'ended'");
    let started = Instant::now();
    let r = rt.eval::<String>(&script);
    let took = started.elapsed();
    assert!(took < scaled(Duration::from_secs(2)), "{value}: {took:?}");
    assert!(
        matches!(&r, Err(DomError::StyleLimit { .. })) || r.as_deref() == Ok("ended"),
        "{value}: {r:?}"
    );
}

/// A block's cost in stylo is its repeats times its size: these took 11 s, 667 ms
/// and 1.3 s in one host call under a 100 ms limit (review of #97). The last two
/// stay under `MAX_DECLARATIONS` and go on `MAX_PUSH_STEPS`.
#[test]
fn a_style_of_repeats_over_the_block() {
    let custom =
        |n: u32| format!("Array.from({{length: {n}}}, (_, i) => '--a' + i + ':;').join('')");
    for value in [
        format!("{} + 'all:inherit;'.repeat(8000)", custom(8000)),
        format!("{} + 'all:inherit;'.repeat(2048)", custom(2048)),
        format!("{} + '\\\\61ll:inherit;'.repeat(2048)", custom(2048)),
        format!("{} + 'ALL:inherit;'.repeat(2048)", custom(2048)),
        format!("{0} + {0}", custom(10000)),
    ] {
        refuse_on_every_write(&value);
    }
}

/// `grid-template-areas` tokenizes its strings again: 150,000 names, 1 MB,
/// took 787 ms under a 100 ms limit (review of #97).
#[test]
fn a_style_of_many_grid_areas() {
    let names = |n: u32| format!("Array.from({{length: {n}}}, (_, i) => 'n' + i).join(' ')");
    for property in ["grid-template-areas", "grid-template", "grid"] {
        for strings in [
            format!("\"'\" + {} + \"'\"", names(150_000)),
            format!("\"'a' \".repeat(5000) + \"'\" + {} + \"'\"", names(100)),
        ] {
            let value = format!("'{property}:' + {strings}");
            refuse_on_every_write(&value);
            style_refused(
                STYLED,
                &strings,
                &format!("document.getElementById('p').style.setProperty('{property}', v)"),
            );
        }
    }
    // Declarations are charged apart, and other strings not at all.
    let script = format!(
        "const p = document.getElementById('p');
        const big = \"'\" + {} + \"'\";
        p.setAttribute('style', 'grid-template-areas:' + big);
        const one = p.style.gridTemplateAreas !== '';
        p.setAttribute('style', 'content:' + big + ';--x:' + big + ';--y:' + big);
        String(one && p.style.getPropertyValue('--x') !== '')",
        names(4000),
    );
    assert!(bounded(STYLED, &script).is_none_or(|s| s == "true"));
    let script = format!(
        "const p = document.getElementById('p');
        p.setAttribute('style', \"--x:'\" + {} + \"'\"); String(p.getAttribute('style').length > 1e6)",
        names(150_000),
    );
    assert!(bounded(STYLED, &script).is_none_or(|s| s == "true"));
}

/// The longest a native can run past a time limit on `style`: each script
/// writes or reads the worst values [`fits`] lets through until the limit, and
/// prints by how much it was missed. For a release build.
/// `--release --test natives -- --ignored --nocapture overrun`
#[test]
#[ignore = "timings, for a release build"]
fn style_overrun_in_release() {
    let list = "'font-family:' + 'a,'.repeat(32000) + 'a'";
    let long = "'--x:\"' + 'a'.repeat(8e6) + '\"'";
    let unique = "Array.from({length: 2047}, (_, i) => '--a' + i + ':;').join('')";
    let rounds = format!("{unique} + 'all:inherit;'.repeat(16)");
    // The review of #97's cases, refused before they get anywhere.
    let custom = "Array.from({length: 8000}, (_, i) => '--a' + i + ':;').join('')";
    let names = "Array.from({length: 150000}, (_, i) => 'n' + i).join(' ')";
    for value in [
        format!("const v = {custom} + 'all:inherit;'.repeat(8000);"),
        format!("const v = 'grid-template-areas:\"' + {names} + '\"';"),
    ] {
        let p = "document.getElementById('p')";
        for step in [
            format!("{p}.setAttribute('style', v)"),
            format!("{p}.innerHTML = '<b style=\\'' + v + '\\'>'"),
        ] {
            let limit = Duration::from_secs(5);
            let mut rt = DomRuntime::new(limits().with_time(limit)).unwrap();
            rt.load(Document::parse(STYLED).unwrap());
            let _: () = rt.eval("0").unwrap();
            let r = rt.eval::<()>(&format!("{value} globalThis.v = v"));
            assert_eq!(r, Ok(()));
            let started = Instant::now();
            let r = rt.eval::<()>(&step);
            println!("refused  {:9.1?} {r:?}", started.elapsed());
        }
    }
    for (name, value) in [("list", list), ("string", long), ("repeats", &rounds)] {
        let p = "document.getElementById('p')";
        let v = format!("const v = {value}; const w = v + ' ';");
        for (path, step) in [
            (
                "setAttribute",
                format!("{p}.setAttribute('style', i % 2 ? v : w)"),
            ),
            (
                "innerHTML",
                format!("{p}.innerHTML = '<b style=\\'' + v + '\\'>'"),
            ),
            (
                "cloneNode",
                format!("{p}.setAttribute('style', v); {p}.cloneNode()"),
            ),
            (
                "read",
                format!("{p}.setAttribute('style', v); {p}.style.length"),
            ),
        ] {
            let limit = Duration::from_millis(100);
            let mut rt = DomRuntime::new(limits().with_time(limit)).unwrap();
            rt.load(Document::parse(STYLED).unwrap());
            let _: () = rt.eval("0").unwrap();
            let started = Instant::now();
            let r = rt.eval::<()>(&format!("{v} for (let i = 0;; i++) {{ {step} }}"));
            let took = started.elapsed();
            println!("{name:8} {path:13} {took:9.1?} {r:?}");
        }
    }
}

#[test]
fn a_large_data_url_is_one_token() {
    let script = "const p = document.getElementById('p');
        const url = 'url(data:image/png;base64,' + 'A'.repeat(6 << 20) + ')';
        p.setAttribute('style', 'background-image:' + url);
        String(p.getAttribute('style').length > 6e6)";
    assert!(bounded(STYLED, script).is_none_or(|n| n == "true"));
}
