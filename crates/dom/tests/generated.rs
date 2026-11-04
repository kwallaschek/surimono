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
