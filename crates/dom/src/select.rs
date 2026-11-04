//! Selector matching that a time limit can stop.
//!
//! One selector match can take far longer than one script instruction:
//! sibling combinators inside `:not()` make a single element's match
//! polynomial in a wide tree's size, in a call the sandbox can't
//! interrupt. So every element goes through [`Guarded`], which delegates to
//! blitz-dom's `selectors::Element` for `&Node` and charges a clock for the
//! work, not only for navigation: each element's match for the selector
//! list's length, each sibling scanned, each child `:empty` reads, and each
//! attribute test for what it reads. Once the deadline passes each step
//! answers "no such element" or "no match", the match ends quickly, and
//! [`Matcher::stopped`] tells the caller.
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
use surimono_core::Deadline;

use crate::document::is_fragment_name;

/// Units of work between looks at the clock.
const STEPS_PER_CHECK: usize = 4096;

/// The clock shared by one query.
struct Budget {
    deadline: Option<Deadline>,
    steps: Cell<usize>,
    stopped: Cell<bool>,
    /// Where [`index_in`](Self::index_in) last found a node.
    hint: Cell<usize>,
}

impl Budget {
    /// Whether matching should give up; counts one step.
    fn over(&self) -> bool {
        self.charge(1)
    }

    /// Whether matching should give up; counts `work` steps.
    fn charge(&self, work: usize) -> bool {
        if self.stopped.get() {
            return true;
        }
        let steps = self.steps.get().saturating_add(work);
        if steps < STEPS_PER_CHECK {
            self.steps.set(steps);
            return false;
        }
        self.steps.set(0);
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
        if self.budget.over() {
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
        .is_some_and(|e| is_fragment_name(&e.name))
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
                return Some(Self {
                    node,
                    budget: self.budget,
                });
            }
        }
        None
    }

    /// Selectors 4: names are ASCII case-insensitive only for HTML elements
    /// in an HTML document, and every document here is an HTML one.
    fn is_html_element_in_html_document(&self) -> bool {
        self.node
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
        self.node
            .parent
            .is_some_and(|p| matches!(self.node.with(p).data, NodeData::Document))
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
    pub(crate) fn new(quirks: QuirksMode, deadline: Option<Deadline>) -> Self {
        Self {
            caches: SelectorCaches::default(),
            budget: Budget {
                deadline,
                steps: Cell::new(0),
                stopped: Cell::new(false),
                hint: Cell::new(0),
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
        if self.budget.charge(list.slice().len()) {
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

    /// Whether the time limit passed, also between elements.
    pub(crate) fn stopped(&self) -> bool {
        self.budget.over()
    }
}
