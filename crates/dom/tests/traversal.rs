//! `NodeIterator` and `TreeWalker` against DOM §6. Expected values are the standard's
//! algorithms worked by hand on these trees, as WPT's dom/traversal does.

mod common;

use common::limits;
use surimono_core::SandboxError;
use surimono_dom::{Document, DomError, DomLimits, DomRuntime};

const TREE: &str =
    "<!doctype html><div id=r><a id=a>t1<b id=b>t2</b></a><!--c--><i id=i></i>t3</div>";

fn runtime(page: &str) -> DomRuntime {
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse(page).unwrap());
    rt
}

fn run(script: &str) -> String {
    runtime(TREE)
        .eval(&format!(
            "const r = document.getElementById('r');
             const name = (n) => n === null ? 'null' : (n.id || n.nodeName === '#text' && n.data || n.nodeName);
             const all = (f) => {{ const o = []; for (let n; (n = f()) !== null;) o.push(name(n)); return o.join(); }};
             {script}"
        ))
        .unwrap()
}

#[test]
fn an_iterator_walks_in_tree_order() {
    assert_eq!(
        run("const i = document.createNodeIterator(r);
             const f = all(() => i.nextNode());
             const b = all(() => i.previousNode());
             [f, b, i.root === r, i.whatToShow >>> 0, i.filter, i.pointerBeforeReferenceNode, name(i.referenceNode),
              i instanceof NodeIterator].join(' ')"),
        "r,a,t1,b,t2,#comment,i,t3 t3,i,#comment,t2,b,t1,a,r true 4294967295  true r true"
    );
    assert_eq!(
        run("const e = document.createNodeIterator(r, NodeFilter.SHOW_ELEMENT);
             const t = document.createNodeIterator(r, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_COMMENT);
             all(() => e.nextNode()) + ' ' + all(() => t.nextNode())"),
        "r,a,b,i t1,t2,#comment,t3"
    );
}

#[test]
fn filters_take_every_form_the_standard_gives() {
    assert_eq!(
        run("const skipB = (n) => n.id === 'b' ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT;
             const i = document.createNodeIterator(r, NodeFilter.SHOW_ALL, skipB);
             const seen = [];
             const o = { acceptNode(n) { seen.push(this === o); return NodeFilter.FILTER_ACCEPT; } };
             const j = document.createNodeIterator(r, 1, o);
             j.nextNode();
             [all(() => i.nextNode()), seen.join(), j.filter === o].join(' ')"),
        "r,a,t1,t2,#comment,i,t3 true true"
    );
    assert_eq!(
        run("const thrown = (f) => { try { f(); return 'none'; } catch (e) { return e.name; } };
             const j = document.createNodeIterator(r, 0xffffffff, {});
             const k = document.createNodeIterator(r, 0xffffffff, () => { throw new RangeError('x'); });
             const again = document.createNodeIterator(r, 0xffffffff, () => { again.nextNode(); return 1; });
             [thrown(() => j.nextNode()), thrown(() => k.nextNode()), thrown(() => again.nextNode()),
              thrown(() => document.createTreeWalker(null)), thrown(() => document.createNodeIterator())].join()"),
        "TypeError,RangeError,InvalidStateError,TypeError,TypeError"
    );
}

#[test]
fn an_iterator_survives_removals() {
    assert_eq!(
        run("const el = (x) => document.getElementById(x);
             const i = document.createNodeIterator(r);
             i.nextNode(); i.nextNode();
             const log = [name(i.referenceNode), i.pointerBeforeReferenceNode];
             el('a').remove();
             log.push(name(i.referenceNode), i.pointerBeforeReferenceNode);
             log.join()"),
        "a,false,r,false"
    );
    assert_eq!(
        run("const el = (x) => document.getElementById(x);
             const i = document.createNodeIterator(r);
             i.nextNode(); i.nextNode(); i.previousNode();
             const log = [name(i.referenceNode), i.pointerBeforeReferenceNode];
             el('a').remove();
             log.push(name(i.referenceNode), i.pointerBeforeReferenceNode);
             log.join()"),
        "a,true,#comment,true"
    );
    // On `b`, pointer after: the node before it in the tree stays.
    assert_eq!(
        run("const el = (x) => document.getElementById(x);
             const i = document.createNodeIterator(r, 1);
             i.nextNode(); i.nextNode(); i.nextNode();
             const log = [name(i.referenceNode)];
             el('a').removeChild(el('b'));
             log.push(name(i.referenceNode));
             log.join()"),
        "b,t1"
    );
}

/// §6.1: nothing moves when what leaves holds the iterator's root.
#[test]
fn an_iterator_inside_what_leaves_stays() {
    assert_eq!(
        run("const a = document.getElementById('a');
             const i = document.createNodeIterator(document.getElementById('b'));
             i.nextNode(); i.nextNode();
             a.remove();
             [name(i.referenceNode), i.pointerBeforeReferenceNode].join()"),
        "t2,false"
    );
}

#[test]
fn an_iterator_survives_removing_all_children() {
    assert_eq!(
        run("const i = document.createNodeIterator(r);
             for (let k = 0; k < 7; k++) i.nextNode();
             const log = [name(i.referenceNode)];
             r.textContent = '';
             log.push(name(i.referenceNode), i.pointerBeforeReferenceNode);
             log.join()"),
        "i,r,false"
    );
    assert_eq!(
        run("const i = document.createNodeIterator(r);
             i.nextNode(); i.nextNode(); i.previousNode();
             r.innerHTML = '<p id=p></p>';
             [name(i.referenceNode), i.pointerBeforeReferenceNode].join()"),
        "r,false"
    );
}

#[test]
fn a_walker_goes_every_way() {
    assert_eq!(
        run("const w = document.createTreeWalker(r);
             const f = all(() => w.nextNode());
             const b = all(() => w.previousNode());
             [f, b, name(w.currentNode), w instanceof TreeWalker].join(' ')"),
        "a,t1,b,t2,#comment,i,t3 i,#comment,t2,b,t1,a,r r true"
    );
    assert_eq!(
        run("const w = document.createTreeWalker(r, NodeFilter.SHOW_ELEMENT);
             const log = [name(w.firstChild()), name(w.firstChild()), name(w.firstChild()), name(w.parentNode()),
                          name(w.nextSibling()), name(w.nextSibling()), name(w.previousSibling()), name(w.lastChild()),
                          name(w.parentNode()), name(w.parentNode())];
             log.join()"),
        "a,b,null,a,i,null,a,b,a,r"
    );
    assert_eq!(
        run("const w = document.createTreeWalker(r);
             w.currentNode = document.getElementById('b');
             const log = [name(w.currentNode), name(w.parentNode()), name(w.lastChild())];
             w.currentNode = document.body;
             log.push(name(w.parentNode()), name(w.nextNode()));
             log.join()"),
        "b,a,b,HTML,HEAD"
    );
}

#[test]
fn a_walker_skips_and_rejects() {
    // FILTER_SKIP looks through a node; FILTER_REJECT leaves its subtree out.
    assert_eq!(
        run(
            "const skip = document.createTreeWalker(r, 1, (n) => n.id === 'a' ? 3 : 1);
             const reject = document.createTreeWalker(r, 1, (n) => n.id === 'a' ? 2 : 1);
             const first = name(skip.firstChild());
             const next = name(skip.nextSibling());
             [first, next, all(() => reject.nextNode())].join(' ')"
        ),
        "b i i"
    );
}

#[test]
fn a_filter_that_changes_the_tree_is_safe() {
    assert_eq!(
        run(
            "const i = document.createNodeIterator(r, 0xffffffff, (n) => {
               if (n.id === 'a') r.textContent = '';
               return 1;
             });
             const w = document.createTreeWalker(r, 0xffffffff, (n) => { n.remove(); return 3; });
             const a = all(() => i.nextNode());
             w.currentNode = r;
             r.append('x', 'y');
             const b = all(() => w.nextNode());
             a + ' ' + b"
        ),
        "r,a "
    );
}

/// A filter that loops forever is the script's own: the limit ends it.
#[test]
fn a_long_walk_ends_with_the_limit() {
    let l = common::scaled_limits(
        surimono_core::Limits::default().with_time(std::time::Duration::from_millis(300)),
    );
    let mut rt = DomRuntime::new(l).unwrap();
    rt.load(Document::parse(TREE).unwrap());
    let e = rt
        .eval::<()>(
            "const w = document.createTreeWalker(document);
             for (;;) { while (w.nextNode()); while (w.previousNode()); }",
        )
        .unwrap_err();
    assert!(
        matches!(e, DomError::Sandbox(SandboxError::TimeLimit)),
        "{e:?}"
    );
}

/// Pointers live outside the heap limit until the document drops.
#[test]
fn iterators_are_charged() {
    let l = DomLimits {
        max_text_bytes: 1024 * 1024,
        ..DomLimits::default()
    };
    let mut rt = DomRuntime::new(limits()).unwrap();
    rt.load(Document::parse_with_limits(TREE, l).unwrap());
    let e = rt
        .eval::<()>("for (;;) document.createNodeIterator(document.body);")
        .unwrap_err();
    assert!(matches!(e, DomError::TextLimit { .. }), "{e:?}");
}

#[test]
fn iterators_go_with_the_document() {
    let mut rt = runtime(TREE);
    let () = rt
        .eval("for (let i = 0; i < 100; i++) document.createNodeIterator(document.body); undefined")
        .unwrap();
    rt.take().unwrap().check_consistency().unwrap();
}

#[test]
fn replacing_dom_members_changes_nothing() {
    assert_eq!(
        run("const w = document.createTreeWalker(r, 1);
             delete Node.prototype.parentNode; delete Node.prototype.firstChild; delete Node.prototype.nextSibling;
             Object.defineProperty(Node.prototype, 'nodeType', { get() { return 1; } });
             const got = TreeWalker.prototype.nextNode.call(w);
             got.getAttribute('id')"),
        "a"
    );
}
