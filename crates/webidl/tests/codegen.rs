//! Generated stubs: golden (checked-in output is current), deterministic,
//! and usable as written.

use surimono_webidl::runtime::{BindingError, ObjectId};
use surimono_webidl::stubs::{self, DOMTokenList, Document, Element, EventTarget, Node};
use surimono_webidl::vendored::FILES;
use surimono_webidl::{IdlSet, STUB_INTERFACES, generate_stubs};

/// `tests/golden/mini.rs` refers to `crate::runtime`; give it one.
mod runtime {
    pub use surimono_webidl::runtime::*;
}

#[path = "golden/mini.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
mod mini;

const REGEN: &str =
    "out of date: run `cargo run -p surimono-webidl --example regen` and review the diff";

#[test]
fn checked_in_stubs_are_current() {
    let fresh = surimono_webidl::generate_vendored_stubs().unwrap();
    assert!(
        fresh == include_str!("../src/stubs.rs"),
        "src/stubs.rs is {REGEN}"
    );
}

#[test]
fn mini_golden_is_current() {
    let fresh = surimono_webidl::generate_mini_golden().unwrap();
    assert!(
        fresh == include_str!("golden/mini.rs"),
        "tests/golden/mini.rs is {REGEN}"
    );
}

#[test]
fn generation_is_deterministic() {
    let a = surimono_webidl::generate_vendored_stubs().unwrap();
    let b = surimono_webidl::generate_vendored_stubs().unwrap();
    assert!(a == b);
    // Input order does not matter either.
    let rev = IdlSet::parse_sources(FILES.iter().rev().copied()).unwrap();
    let fwd = IdlSet::parse_sources(FILES.iter().copied()).unwrap();
    let x = generate_stubs(&fwd, STUB_INTERFACES, "h").unwrap();
    let y = generate_stubs(&rev, STUB_INTERFACES, "h").unwrap();
    assert!(x == y);
}

#[test]
fn interface_table_matches_the_list() {
    let names: Vec<&str> = stubs::INTERFACES.iter().map(|i| i.name).collect();
    assert_eq!(names, STUB_INTERFACES);
    let parent = |n: &str| {
        stubs::INTERFACES
            .iter()
            .find(|i| i.name == n)
            .unwrap()
            .parent
    };
    assert_eq!(parent("EventTarget"), None);
    assert_eq!(parent("Node"), Some("EventTarget"));
    assert_eq!(parent("Element"), Some("Node"));
    assert_eq!(parent("Document"), Some("Node"));
    assert_eq!(parent("DOMTokenList"), None);
    assert_eq!(stubs::node_consts::ELEMENT_NODE, 1);
    assert_eq!(stubs::node_consts::DOCUMENT_POSITION_DISCONNECTED, 0x01);
    let mini: Vec<&str> = mini::INTERFACES.iter().map(|i| i.name).collect();
    assert_eq!(mini, ["Base", "Widget"]);
}

/// A host that implements one member and leaves the rest to the defaults.
#[derive(Debug, Default)]
struct Host {
    names: Vec<(ObjectId, String)>,
}

impl EventTarget for Host {}
impl Node for Host {
    fn node_name(&mut self, this: ObjectId) -> Result<String, BindingError> {
        self.names
            .iter()
            .find(|(id, _)| *id == this)
            .map(|(_, n)| n.clone())
            .ok_or(BindingError::TypeError("Illegal invocation".into()))
    }
}
impl Element for Host {}
impl Document for Host {}
impl DOMTokenList for Host {}

#[test]
fn stubs_are_implementable_one_member_at_a_time() {
    let mut host = Host::default();
    host.names.push((ObjectId(7), "DIV".into()));
    assert_eq!(Node::node_name(&mut host, ObjectId(7)).unwrap(), "DIV");
    assert!(matches!(
        Node::node_name(&mut host, ObjectId(8)),
        Err(BindingError::TypeError(_))
    ));
    assert_eq!(
        Element::get_attribute(&mut host, ObjectId(7), "id".into()),
        Err(BindingError::NotImplemented("Element.getAttribute"))
    );
    assert_eq!(
        DOMTokenList::add(&mut host, ObjectId(1), vec!["a".into()]),
        Err(BindingError::NotImplemented("DOMTokenList.add"))
    );
    assert_eq!(
        Document::create_element(
            &mut host,
            ObjectId(0),
            "p".into(),
            surimono_webidl::runtime::Value::Undefined
        ),
        Err(BindingError::NotImplemented("Document.createElement"))
    );
    // Trait objects work: phase 2 can hold the host as `dyn Element`.
    let dynamic: &mut dyn Element = &mut host;
    assert!(dynamic.has_attributes(ObjectId(7)).is_err());
}
