//! The checked-in binding glue is what the generator writes today.

const REGEN: &str =
    "out of date: run `cargo run -p surimono-webidl --example regen` and review the diff";

#[test]
fn generated_bindings_are_current() {
    let fresh = surimono_webidl::generate_vendored_bindings().unwrap();
    assert!(
        fresh.js == include_str!("../src/generated/bindings.js"),
        "bindings.js is {REGEN}"
    );
    assert!(
        fresh.rust == include_str!("../src/generated/dispatch.rs"),
        "dispatch.rs is {REGEN}"
    );
}

/// Each hand-written member the generated code calls exists in the glue: a
/// missing one would only show as "not a function" when a page calls it.
#[test]
fn every_glue_member_is_written() {
    let generated = include_str!("../src/generated/bindings.js");
    let glue = concat!(
        include_str!("../src/glue_events.js"),
        include_str!("../src/glue_abort.js"),
        include_str!("../src/glue_location.js"),
        include_str!("../src/glue_activation.js"),
        include_str!("../src/glue_layout.js"),
        include_str!("../src/glue_tail.js"),
    );
    let mut missing = Vec::new();
    for piece in generated.split("H.").skip(1) {
        let name: String = piece
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && !glue.contains(&format!("H.{name} =")) && !missing.contains(&name) {
            missing.push(name);
        }
    }
    assert!(missing.is_empty(), "{missing:?}");
}
