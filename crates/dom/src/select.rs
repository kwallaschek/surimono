//! Selector matching that a time limit can stop.
//!
//! One selector match can take far longer than one script instruction:
//! sibling combinators inside `:not()` make a single element's match
//! polynomial in a wide tree's size, in a call the sandbox can't
//! interrupt. So every element goes through [`Guarded`], which delegates to
//! blitz-dom's `selectors::Element` for `&Node` and charges a clock for the
//! work, not only for navigation: each element reached for the selector's
//! size, since all of it can be tested there, each sibling scanned, each
//! child `:empty` reads, and each attribute test for what it reads. Once the
//! deadline passes each step
//! answers "no such element" or "no match", the match ends quickly, and
//! [`Matcher::stopped`] tells the caller.
//!
//! The matcher recurses for each combinator, on the stack. The caps on a
//! selector keep that within `Sandbox::STACK_HEADROOM_BYTES`, by measurement;
//! what makes it sure is that reaching an element also looks at the stack
//! left, and below [`MIN_STACK_BYTES`] stops the match the same way and ends
//! the call with [`SandboxError::StackOverflow`](surimono_core::SandboxError).
//! The parser runs first, and its stack is only the caps' to bound.
//!
//! Blitz's own sibling lookups find the node's index by a scan per step, so
//! they aren't used: [`Budget::index_in`] searches out from the last index it
//! found, which costs little in tree order and is charged in any order.
//!
//! Stopping mid-way can leave a wrong count in the nth-child cache, which is
//! fine: the query fails. In debug builds only, a `debug_assert` in
//! `selectors` recounts and can then panic, if the stop falls inside an
//! nth-child count. Release builds don't have that assertion.
//!
//! The same wrapper corrects four answers: a fragment isn't a parent
//! element, `:root` is only the document's element (blitz-dom says yes for
//! any tree's second level), comments and empty text don't stop `:empty`,
//! and only HTML elements are matched case-insensitively (blitz-dom says
//! every element is one, so `linearGradient` could never match).

use std::cell::Cell;
use std::ptr::NonNull;

use blitz_dom::{Node, NodeData, local_name, ns};
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::matching::{
    self, ElementSelectorFlags, MatchingContext, MatchingForInvalidation, MatchingMode,
    NeedsSelectorFlags, SelectorCaches,
};
use selectors::{Element, OpaqueElement, SelectorList};
use style::context::QuirksMode;
use style::selector_parser::SelectorImpl as Impl;
use surimono_core::{Deadline, stack_available};

use crate::document::{is_container_name, is_document_name};

/// Units of work between looks at the clock.
const STEPS_PER_CHECK: usize = 4096;

/// Stack a match must have left to go on to another element. Between two
/// looks there is at most one combinator's recursion, or
/// `MAX_SELECTOR_NESTING` levels of `:is()`, and a leaf test: at most a few
/// KiB a level in an unoptimized build, and the worst selector allowed took
/// under 142 KiB in all (see `MAX_SELECTOR_NESTING`).
const MIN_STACK_BYTES: usize = 64 * 1024;

#[cfg(test)]
thread_local! {
    /// A test's [`MIN_STACK_BYTES`].
    pub(crate) static TEST_MIN_STACK: Cell<usize> = const { Cell::new(MIN_STACK_BYTES) };
}

fn min_stack() -> usize {
    #[cfg(test)]
    return TEST_MIN_STACK.get();
    #[cfg(not(test))]
    MIN_STACK_BYTES
}

/// The clock shared by one query.
struct Budget {
    deadline: Option<Deadline>,
    /// Steps charged so far.
    steps: Cell<usize>,
    /// The step count at which to look at the clock next.
    next_check: Cell<usize>,
    stopped: Cell<bool>,
    /// Where [`index_in`](Self::index_in) last found a node.
    hint: Cell<usize>,
    /// What testing the whole selector on one element costs, at most: see
    /// [`element_weight`].
    weight: usize,
    /// The query is in an HTML document, where HTML elements match
    /// case-insensitively.
    html_document: bool,
}

/// The steps one element's test of a selector of `bytes` bytes costs. Nothing
/// charges for simple selectors, and a list nested in `:is()` is invisible
/// from the top level, so the size of the text stands in: a compound takes
/// at least two bytes with its separator (`x,`).
fn element_weight(bytes: usize) -> usize {
    bytes / 2 + 1
}

impl Budget {
    /// Whether matching should give up; counts one step.
    fn over(&self) -> bool {
        self.charge(1)
    }

    /// Whether matching should give up; counts reaching one element, where
    /// the rest of the selector is tested, and looks at the stack left.
    fn reach(&self) -> bool {
        self.stack_low() || self.charge(self.weight)
    }

    /// Whether the stack is too short to go on; if so, stops the query and
    /// ends the call.
    fn stack_low(&self) -> bool {
        if stack_available().is_none_or(|left| left >= min_stack()) {
            return false;
        }
        self.stopped.set(true);
        if let Some(deadline) = &self.deadline {
            deadline.exceed_stack();
        }
        true
    }

    /// Whether matching should give up; counts `work` steps.
    fn charge(&self, work: usize) -> bool {
        if self.stopped.get() {
            return true;
        }
        let steps = self.steps.get().saturating_add(work);
        self.steps.set(steps);
        if steps < self.next_check.get() {
            return false;
        }
        self.next_check.set(steps.saturating_add(STEPS_PER_CHECK));
        if self.deadline.as_ref().is_some_and(Deadline::passed) {
            self.stopped.set(true);
        }
        self.stopped.get()
    }

    /// The index of `id` in `list`, searching outwards from the last index
    /// found. `None` once stopped.
    fn index_in(&self, list: &[usize], id: usize) -> Option<usize> {
        let hint = self.hint.get().min(list.len().saturating_sub(1));
        for distance in 0..list.len() {
            if self.over() {
                return None;
            }
            let above = hint + distance;
            let below = hint.checked_sub(distance);
            let found = if list.get(above) == Some(&id) {
                Some(above)
            } else {
                below.filter(|&b| list[b] == id)
            };
            if let Some(at) = found {
                self.hint.set(at);
                return Some(at);
            }
        }
        None
    }
}

#[derive(Clone, Copy)]
struct Guarded<'a> {
    node: &'a Node,
    budget: &'a Budget,
}

impl std::fmt::Debug for Guarded<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Guarded({})", self.node.id)
    }
}

impl<'a> Guarded<'a> {
    fn step(&self, next: Option<&'a Node>) -> Option<Self> {
        if self.budget.reach() {
            return None;
        }
        next.map(|node| Self {
            node,
            budget: self.budget,
        })
    }

    /// The nearest element sibling, charging each node passed.
    fn sibling_element(&self, forward: bool) -> Option<Self> {
        let list = &self.node.with(self.node.parent?).children;
        let mut at = self.budget.index_in(list, self.node.id)?;
        loop {
            if self.budget.over() {
                return None;
            }
            at = if forward { at + 1 } else { at.checked_sub(1)? };
            let node = self.node.with(*list.get(at)?);
            if node.is_element() {
                self.budget.hint.set(at);
                if self.budget.reach() {
                    return None;
                }
                return Some(Self {
                    node,
                    budget: self.budget,
                });
            }
        }
    }

    /// Charges a test of attribute `value`, which costs a search of the
    /// element's attributes and a read of the value. Whether to give up.
    fn charge_attribute(&self, value: Option<&str>) -> bool {
        let attributes = self.node.attrs().map_or(0, <[_]>::len);
        self.budget
            .charge(attributes + value.map_or(0, str::len) / 16)
    }
}

fn is_fragment(node: &Node) -> bool {
    node.element_data()
        .is_some_and(|e| is_container_name(&e.name))
}

impl Element for Guarded<'_> {
    type Impl = Impl;

    fn opaque(&self) -> OpaqueElement {
        // The node id, not its address: slab growth moves nodes. Only ever
        // compared, and id + 1 is never null.
        match NonNull::new((self.node.id + 1) as *mut ()) {
            Some(p) => OpaqueElement::from_non_null_ptr(p),
            None => OpaqueElement::from_non_null_ptr(NonNull::<()>::dangling()),
        }
    }

    fn parent_element(&self) -> Option<Self> {
        self.step(Element::parent_element(&self.node).filter(|p| !is_fragment(p)))
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        Element::is_pseudo_element(&self.node)
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.sibling_element(false)
    }

    fn next_sibling_element(&self) -> Option<Self> {
        self.sibling_element(true)
    }

    fn first_element_child(&self) -> Option<Self> {
        for &c in &self.node.children {
            if self.budget.over() {
                return None;
            }
            let node = self.node.with(c);
            if node.is_element() {
                if self.budget.reach() {
                    return None;
                }
                return Some(Self {
                    node,
                    budget: self.budget,
                });
            }
        }
        None
    }

    /// Selectors 4: names are ASCII case-insensitive only for HTML elements
    /// in an HTML document; the query knows which its document is.
    fn is_html_element_in_html_document(&self) -> bool {
        self.budget.html_document
            && self
                .node
                .element_data()
                .is_some_and(|e| e.name.ns == ns!(html))
    }

    fn has_local_name(&self, name: &<Impl as selectors::SelectorImpl>::BorrowedLocalName) -> bool {
        Element::has_local_name(&self.node, name)
    }

    fn has_namespace(&self, ns: &<Impl as selectors::SelectorImpl>::BorrowedNamespaceUrl) -> bool {
        Element::has_namespace(&self.node, ns)
    }

    fn is_same_type(&self, other: &Self) -> bool {
        Element::is_same_type(&self.node, &other.node)
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&<Impl as selectors::SelectorImpl>::NamespaceUrl>,
        local_name: &<Impl as selectors::SelectorImpl>::LocalName,
        operation: &AttrSelectorOperation<&<Impl as selectors::SelectorImpl>::AttrValue>,
    ) -> bool {
        if self.charge_attribute(self.node.data.attr(local_name.0.clone())) {
            return false;
        }
        Element::attr_matches(&self.node, ns, local_name, operation)
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &<Impl as selectors::SelectorImpl>::NonTSPseudoClass,
        context: &mut MatchingContext<'_, Impl>,
    ) -> bool {
        Element::match_non_ts_pseudo_class(&self.node, pc, context)
    }

    fn match_pseudo_element(
        &self,
        pe: &<Impl as selectors::SelectorImpl>::PseudoElement,
        context: &mut MatchingContext<'_, Impl>,
    ) -> bool {
        Element::match_pseudo_element(&self.node, pe, context)
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        Element::is_link(&self.node)
    }

    fn is_html_slot_element(&self) -> bool {
        Element::is_html_slot_element(&self.node)
    }

    fn has_id(
        &self,
        id: &<Impl as selectors::SelectorImpl>::Identifier,
        case_sensitivity: CaseSensitivity,
    ) -> bool {
        Element::has_id(&self.node, id, case_sensitivity)
    }

    fn has_class(
        &self,
        name: &<Impl as selectors::SelectorImpl>::Identifier,
        case_sensitivity: CaseSensitivity,
    ) -> bool {
        if self.charge_attribute(self.node.data.attr(local_name!("class"))) {
            return false;
        }
        Element::has_class(&self.node, name, case_sensitivity)
    }

    fn has_custom_state(&self, _name: &<Impl as selectors::SelectorImpl>::Identifier) -> bool {
        false
    }

    fn imported_part(
        &self,
        _name: &<Impl as selectors::SelectorImpl>::Identifier,
    ) -> Option<<Impl as selectors::SelectorImpl>::Identifier> {
        None
    }

    fn is_part(&self, _name: &<Impl as selectors::SelectorImpl>::Identifier) -> bool {
        false
    }

    /// `:empty`: no children but comments and empty text (Selectors 3, which
    /// browsers follow; a `<style>`'s text is out of blitz-dom's reach, see
    /// [`crate::parse`], and it counts as there).
    fn is_empty(&self) -> bool {
        let style = self
            .node
            .element_data()
            .is_some_and(|e| e.name.local.as_ref() == "style");
        self.node.children.iter().all(|&c| {
            !self.budget.over()
                && match &self.node.with(c).data {
                    NodeData::Comment => true,
                    NodeData::Text(t) => t.content.is_empty() && !style,
                    _ => false,
                }
        })
    }

    /// Selectors 4 `:root`: the element whose parent is the document.
    fn is_root(&self) -> bool {
        self.node.parent.is_some_and(|p| {
            let parent = self.node.with(p);
            matches!(parent.data, NodeData::Document)
                || parent
                    .element_data()
                    .is_some_and(|e| is_document_name(&e.name))
        })
    }

    /// Once stopped, skips the sibling counts altogether.
    fn ignores_nth_child_selectors(&self) -> bool {
        self.budget.stopped.get()
    }

    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }
}

/// Matching state for one query: the nth-child and `:has()` caches, which
/// must outlive one element, and the clock.
pub(crate) struct Matcher {
    caches: SelectorCaches,
    budget: Budget,
    quirks: QuirksMode,
}

impl Matcher {
    /// A matcher for one query of the selector text `selector`.
    pub(crate) fn new(
        quirks: QuirksMode,
        deadline: Option<Deadline>,
        selector: &str,
        html_document: bool,
    ) -> Self {
        Self {
            caches: SelectorCaches::default(),
            budget: Budget {
                deadline,
                steps: Cell::new(0),
                next_check: Cell::new(STEPS_PER_CHECK),
                stopped: Cell::new(false),
                hint: Cell::new(0),
                weight: element_weight(selector.len()),
                html_document,
            },
            quirks,
        }
    }

    /// Whether `node` matches, with `scope` as `:scope`: without one, the
    /// document's element, as for a query on the document (Selectors 4).
    pub(crate) fn matches(
        &mut self,
        node: &Node,
        scope: Option<&Node>,
        list: &SelectorList<Impl>,
    ) -> bool {
        if self.budget.reach() {
            return false;
        }
        let element = Guarded {
            node,
            budget: &self.budget,
        };
        let scope = scope.map(|node| Guarded {
            node,
            budget: &self.budget,
        });
        let mut context = MatchingContext::new(
            MatchingMode::Normal,
            None,
            &mut self.caches,
            self.quirks,
            NeedsSelectorFlags::No,
            MatchingForInvalidation::No,
        );
        context.scope_element = scope.map(|s| s.opaque());
        matching::matches_selector_list(list, &element, &mut context)
    }

    /// The steps charged so far.
    #[cfg(test)]
    pub(crate) fn steps(&self) -> usize {
        self.budget.steps.get()
    }

    /// Whether the time limit passed, also between elements.
    pub(crate) fn stopped(&self) -> bool {
        self.budget.over()
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use super::*;
    use crate::Document;

    /// Goes down the stack until `left` bytes remain, then runs `f`.
    #[inline(never)]
    fn with_stack_left(left: usize, f: &mut dyn FnMut()) {
        let pad = black_box([0u8; 1024]);
        if stack_available().unwrap() > left {
            with_stack_left(left, f);
        } else {
            f();
        }
        black_box(pad);
    }

    /// The real thing, with no JavaScript in the way: stack for a few of the
    /// 64 combinators and not for all, and the match stops above the floor.
    #[test]
    fn a_match_short_of_stack_stops_before_it_overflows() {
        if stack_available().is_none() {
            return;
        }
        let page = format!("<!doctype html><body>{}", "<b></b>".repeat(100));
        let selector = format!("b{}", "+b".repeat(64));
        let worker = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(move || {
                let doc = Document::parse(&page).unwrap();
                let list = doc.selector_list(&selector).unwrap();
                let body = doc.body().unwrap();
                let last = *doc.children(body).last().unwrap();
                let run = |matcher: &mut Matcher| {
                    let found = matcher.matches(doc.node(last), None, &list);
                    (found, matcher.stopped())
                };
                let new = || Matcher::new(QuirksMode::NoQuirks, None, &selector);
                assert_eq!(run(&mut new()), (true, false));
                let (mut stopped, mut lowest) = (false, usize::MAX);
                with_stack_left(MIN_STACK_BYTES + 4 * 1024, &mut || {
                    stopped = run(&mut new()) == (false, true);
                    lowest = stack_available().unwrap();
                });
                assert!(stopped, "{lowest}");
            })
            .unwrap();
        worker.join().expect("no stack overflow");
    }

    /// It ends the call with the typed error, however the script takes the
    /// exception, on all three queries; `TEST_MIN_STACK` stands in for a stack
    /// nearly gone, which the caps don't let a script reach.
    #[test]
    fn running_out_of_stack_ends_the_call_typed() {
        use surimono_core::{Limits, SandboxError};

        use crate::{DomError, DomRuntime};

        for query in [
            "document.querySelector('b')",
            "document.body.firstChild.matches('b')",
            "document.body.firstChild.closest('b')",
        ] {
            let mut rt = DomRuntime::new(Limits::default()).unwrap();
            rt.load(Document::parse("<!doctype html><body><b></b>").unwrap());
            rt.eval::<()>("globalThis.log = 'not run'").unwrap();
            TEST_MIN_STACK.set(usize::MAX);
            let run = rt.eval::<i32>(&format!("try {{ {query}; }} catch (e) {{}} 1"));
            TEST_MIN_STACK.set(MIN_STACK_BYTES);
            assert_eq!(
                run,
                Err(DomError::Sandbox(SandboxError::StackOverflow)),
                "{query}"
            );
            // The state went with it, and the runtime works.
            assert_eq!(rt.eval::<String>("typeof log").unwrap(), "undefined");
            assert_eq!(rt.eval::<bool>("!!document.querySelector('b')"), Ok(true));
        }
    }
}
