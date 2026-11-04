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
fn a_long_list_inside_is_on_many_elements() {
    // The top-level list has one entry; the work is in the nested one.
    let page = format!("<!doctype html>{}", "<b></b>".repeat(20_000));
    let script = "String(document.querySelectorAll(':is(' + 'x,'.repeat(65000) + 'x)').length)";
    assert_eq!(bounded(&page, script), None, "20,000 × 65,000 can't finish");
}

#[test]
fn a_long_list_inside_is_tested_on_each_sibling() {
    // One element's match, so the work is all in the steps to its siblings.
    let page = format!("<!doctype html><body>{}", "<b></b>".repeat(20_000));
    let script = "String(document.body.lastChild.matches(
        ':is(' + 'x,'.repeat(65000) + 'x) ~ b'))";
    assert_eq!(bounded(&page, script), None, "20,000 × 65,000 can't finish");
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
