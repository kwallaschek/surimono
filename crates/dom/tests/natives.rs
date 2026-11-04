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

/// [`bounded`] for what must end in `DomError::StyleLimit`, before stylo: the
/// caps don't wait for the deadline, so the call is over well within it.
fn style_refused(page: &str, script: &str) {
    let mut rt = DomRuntime::new(limits().with_time(scaled(LIMIT))).unwrap();
    rt.load(Document::parse(page).unwrap());
    let _: () = rt.eval("0").unwrap();
    let started = Instant::now();
    let r = rt.eval::<()>(script);
    let took = started.elapsed();
    assert!(
        matches!(r, Err(DomError::StyleLimit { .. })),
        "{script}: {r:?}"
    );
    assert!(took < scaled(Duration::from_secs(2)), "{script}: {took:?}");
}

const STYLED: &str = r#"<!doctype html><body><p id="p"></p>"#;

#[test]
fn a_huge_font_family_through_set_property() {
    // 8 MiB of tokens took 1.12 s in a release build, 2.78 s in a debug one,
    // under a 300 ms limit (review of #97).
    style_refused(
        STYLED,
        "document.getElementById('p').style.setProperty('font-family', 'a,'.repeat(4e6) + 'a')",
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
        style_refused(STYLED, &format!("{p}.setAttribute('style', {value})"));
        style_refused(
            STYLED,
            &format!("{p}.setAttributeNS('urn:x', 'style', {value})"),
        );
    }
    // Just over the cap, or feeding the parser itself runs into the limit.
    let value = "'font-family:' + 'a,'.repeat(40000) + 'a'";
    for html in [
        format!("'<b style=\"' + {value} + '\">'"),
        format!("'<template><b style=\"' + {value} + '\"></template>'"),
    ] {
        style_refused(STYLED, &format!("{p}.innerHTML = {html}"));
    }
}

#[test]
fn deep_brackets_in_a_style_attribute() {
    let p = "document.getElementById('p')";
    let value = "'background: ' + 'a('.repeat(33)";
    style_refused(STYLED, &format!("{p}.setAttribute('style', {value})"));
    style_refused(
        STYLED,
        &format!("{p}.innerHTML = '<b style=\"' + {value} + '\">'"),
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

#[test]
fn a_large_data_url_is_one_token() {
    let script = "const p = document.getElementById('p');
        const url = 'url(data:image/png;base64,' + 'A'.repeat(6 << 20) + ')';
        p.setAttribute('style', 'background-image:' + url);
        String(p.getAttribute('style').length > 6e6)";
    assert!(bounded(STYLED, script).is_none_or(|n| n == "true"));
}
