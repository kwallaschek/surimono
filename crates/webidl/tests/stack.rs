//! Parsing, printing and generating from hostile or deeply nested IDL fits a
//! 256 KiB thread stack: nesting is capped (32 levels in the parser, 32 in
//! type resolution), so a small-stack worker gets an error, never a stack
//! overflow.

use surimono_webidl::{IdlSet, generate_stubs, parse};

const SMALL_STACK: usize = 256 * 1024;

/// Runs `f` on a thread with a 256 KiB stack. A stack overflow aborts the
/// whole test binary, so passing means it fit.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .name("small-stack".into())
        .stack_size(SMALL_STACK)
        .spawn(f)
        .expect("spawn a 256 KiB thread")
        .join()
        .expect("small-stack thread panicked")
}

/// Every recursive path through the parser, nested far past the cap.
fn hostile() -> Vec<(&'static str, String)> {
    const N: usize = 10_000;
    vec![
        (
            "ext attr args",
            format!("{}interface A {{}};", "[A(".repeat(N)),
        ),
        (
            "named args",
            format!("{}interface A {{}};", "[A=B(".repeat(N)),
        ),
        (
            "sequence",
            format!("typedef {}long T;", "sequence<".repeat(N)),
        ),
        ("union", format!("typedef {}long T;", "(".repeat(N))),
        (
            "annotated sequence",
            format!("typedef {}long T;", "[X] sequence<".repeat(N)),
        ),
        (
            "record",
            format!("typedef {}long T;", "record<DOMString, ".repeat(N)),
        ),
        (
            "args then types",
            format!("{}interface A {{}};", "[A([B] sequence<".repeat(N)),
        ),
        (
            "optional union args",
            format!(
                "{}interface A {{}};",
                "[A(optional [B] (sequence<".repeat(N)
            ),
        ),
        (
            "promise in operation",
            format!(
                "interface A {{ {}long f(); }};",
                "Promise<FrozenArray<".repeat(N)
            ),
        ),
    ]
}

#[test]
fn hostile_nesting_is_an_error_on_a_256_kib_stack() {
    let results = on_small_stack(|| {
        hostile()
            .into_iter()
            .map(|(what, src)| (what, parse(&src).is_err()))
            .collect::<Vec<_>>()
    });
    for (what, is_err) in results {
        assert!(is_err, "{what}: expected a parse error");
    }
}

#[test]
fn deepest_accepted_idl_round_trips_and_generates_on_a_256_kib_stack() {
    on_small_stack(|| {
        // 31 `sequence<` plus the element type is 32 levels, the parser's cap.
        let deep = format!("{}long{}", "sequence<".repeat(31), ">".repeat(31));
        let union = (0..31).fold("long".to_owned(), |u, _| format!("({u} or short)"));
        let src = format!(
            "typedef {deep} T0;\n\
             typedef {union} U0;\n\
             typedef T0 T1;\n\
             interface A {{ attribute U0 u; undefined f(sequence<T1> x); }};\n"
        );
        let defs = parse(&src).expect("deep but within the cap");
        let printed: String = defs.iter().map(|d| format!("{d}\n")).collect();
        let again = parse(&printed).expect("reprinted");
        assert_eq!(defs.len(), again.len());
        let set = IdlSet::parse_sources([("deep.idl", src.as_str())]).expect("set");
        // Type resolution follows the typedefs on top of the parser's
        // nesting; past its own cap it stops with `TooDeep`, not an overflow.
        let _ = generate_stubs(&set, &["A"], "deep");
        // A typedef chain longer than the resolver's cap is `TooDeep`.
        let chain: String = (0..1000)
            .map(|i| format!("typedef T{} T{};\n", i, i + 1))
            .chain(["typedef long T0;\ninterface B { attribute T1000 x; };\n".to_owned()])
            .collect();
        let set = IdlSet::parse_sources([("chain.idl", chain.as_str())]).expect("chain set");
        assert!(generate_stubs(&set, &["B"], "chain").is_err());
        drop(defs);
        drop(again);
    });
}
