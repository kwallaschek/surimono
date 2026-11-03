//! Selector matching that a time limit can stop.
//!
//! One selector match can take far longer than one script instruction:
//! sibling combinators and `:has()` inside `:not()` make a single element's
//! match polynomial in a wide tree's size, in a call the sandbox can't
//! interrupt. So every element goes through [`Guarded`], which delegates to
//! blitz-dom's `selectors::Element` for `&Node` and counts navigation steps;
//! once the deadline passes each step answers "no such element", the match
//! ends quickly, and [`Matcher::stopped`] tells the caller.
//!
//! Stopping mid-way can leave a wrong count in the nth-child cache, which is
//! fine: the query fails. In debug builds only, a `debug_assert` in
//! `selectors` recounts and can then panic, if the stop falls inside an
//! nth-child count. Release builds don't have that assertion.
//!
//! The same wrapper corrects three answers: a fragment isn't a parent
//! element, `:root` is only the document's element (blitz-dom says yes for
//! any tree's second level), and comments don't stop `:empty`.

use std::cell::Cell;
use std::ptr::NonNull;

use blitz_dom::{Node, NodeData};
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

/// Navigation steps between looks at the clock.
const STEPS_PER_CHECK: u32 = 4096;

/// The clock shared by one query.
struct Budget {
    deadline: Option<Deadline>,
    steps: Cell<u32>,
    stopped: Cell<bool>,
}

impl Budget {
    /// Whether matching should give up; counts one step.
    fn over(&self) -> bool {
        if self.stopped.get() {
            return true;
        }
        let s = self.steps.get().wrapping_add(1);
        self.steps.set(s);
        if s % STEPS_PER_CHECK == 0 && self.deadline.as_ref().is_some_and(Deadline::passed) {
            self.stopped.set(true);
        }
        self.stopped.get()
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
        self.step(Element::prev_sibling_element(&self.node))
    }

    fn next_sibling_element(&self) -> Option<Self> {
        self.step(Element::next_sibling_element(&self.node))
    }

    fn first_element_child(&self) -> Option<Self> {
        self.step(Element::first_element_child(&self.node))
    }

    fn is_html_element_in_html_document(&self) -> bool {
        Element::is_html_element_in_html_document(&self.node)
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

    /// Selectors 4 `:empty`: comments don't count.
    fn is_empty(&self) -> bool {
        self.node
            .children
            .iter()
            .all(|&c| matches!(self.node.with(c).data, NodeData::Comment))
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
            },
            quirks,
        }
    }

    /// Whether `node` matches, with `scope` as `:scope`.
    pub(crate) fn matches(&mut self, node: &Node, scope: &Node, list: &SelectorList<Impl>) -> bool {
        let element = Guarded {
            node,
            budget: &self.budget,
        };
        let scope = Guarded {
            node: scope,
            budget: &self.budget,
        };
        let mut context = MatchingContext::new(
            MatchingMode::Normal,
            None,
            &mut self.caches,
            self.quirks,
            NeedsSelectorFlags::No,
            MatchingForInvalidation::No,
        );
        context.scope_element = Some(scope.opaque());
        matching::matches_selector_list(list, &element, &mut context)
    }

    /// Whether the time limit passed, also between elements.
    pub(crate) fn stopped(&self) -> bool {
        self.budget.over()
    }
}
