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
//!
//! It also answers the pseudo-classes blitz-dom leaves false, all of which a
//! page's own scripts ask about forms: `:enabled`, `:disabled`, `:checked`,
//! `:required`, `:optional`, `:read-only`, `:read-write`, `:lang()`, `:defined`
//! and `:target` (HTML §4.16.3, Selectors 4), and honours the `i` flag of
//! attribute selectors. Walks up the tree and along an option's siblings are
//! charged like any other.

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
use style::selector_parser::NonTSPseudoClass;
use style::selector_parser::SelectorImpl as Impl;
use surimono_core::Deadline;

use crate::document::{is_container_name, is_document_name};
use crate::host::custom_element_name;

/// Units of work between looks at the clock.
const STEPS_PER_CHECK: usize = 4096;

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
    /// The document's target element (`:target`), if the query asks.
    target: Option<usize>,
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
    /// the rest of the selector is tested.
    fn reach(&self) -> bool {
        self.charge(self.weight)
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

    /// The HTML element name, if this is one.
    fn html_name(&self) -> Option<&str> {
        let e = self.node.element_data()?;
        (e.name.ns == ns!(html)).then_some(e.name.local.as_ref())
    }

    fn has(&self, name: blitz_dom::LocalName) -> bool {
        self.node.attr(name).is_some()
    }

    /// The parent, if it is an element, charged as a step up.
    fn up(&self) -> Option<Self> {
        self.step(Element::parent_element(&self.node).filter(|p| !is_fragment(p)))
    }

    /// `fieldset`'s first `legend` child, if `child` is it.
    fn is_first_legend(&self, child: &Self) -> bool {
        if child.html_name() != Some("legend") {
            return false;
        }
        for &c in &self.node.children {
            if self.budget.over() {
                return false;
            }
            let node = self.node.with(c);
            let found = Self {
                node,
                budget: self.budget,
            };
            if found.html_name() == Some("legend") {
                return c == child.node.id;
            }
        }
        false
    }

    /// In a `fieldset` with `disabled`, outside its first `legend`.
    fn in_disabled_fieldset(&self) -> bool {
        let mut child = *self;
        while let Some(at) = child.up() {
            if at.html_name() == Some("fieldset")
                && at.has(local_name!("disabled"))
                && !at.is_first_legend(&child)
            {
                return true;
            }
            child = at;
        }
        false
    }

    /// HTML §4.16.3 "actually disabled": `None` if this isn't an element
    /// that can be disabled at all.
    fn actually_disabled(&self) -> Option<bool> {
        let name = self.html_name()?;
        let own = self.has(local_name!("disabled"));
        Some(match name {
            "button" | "input" | "select" | "textarea" | "fieldset" => {
                own || self.in_disabled_fieldset()
            }
            "optgroup" => own,
            "option" => {
                own || self.up().is_some_and(|p| {
                    p.html_name() == Some("optgroup") && p.has(local_name!("disabled"))
                })
            }
            _ => return None,
        })
    }

    fn input_type(&self) -> String {
        self.node
            .attr(local_name!("type"))
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    /// An option with no `selected` is selected when it is the first option
    /// that isn't disabled of a `select` that shows one (HTML §4.10.7's "ask
    /// for a reset"), unless another one has `selected`.
    fn option_selected(&self) -> bool {
        if self.has(local_name!("selected")) {
            return true;
        }
        let mut select = self.up();
        if select.is_some_and(|p| p.html_name() == Some("optgroup")) {
            select = select.and_then(|p| p.up());
        }
        let Some(select) = select.filter(|p| p.html_name() == Some("select")) else {
            return false;
        };
        let size = select.node.attr(local_name!("size"));
        let display_size = size.and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
        if select.has(local_name!("multiple")) || display_size > 1 {
            return false;
        }
        let mut stack: Vec<usize> = select.node.children.iter().rev().copied().collect();
        let mut first = None;
        while let Some(id) = stack.pop() {
            if self.budget.over() {
                return false;
            }
            let at = Self {
                node: self.node.with(id),
                budget: self.budget,
            };
            match at.html_name() {
                Some("option") => {
                    if at.has(local_name!("selected")) {
                        return false;
                    }
                    if first.is_none() && at.actually_disabled() == Some(false) {
                        first = Some(id);
                    }
                }
                Some("optgroup") => stack.extend(at.node.children.iter().rev()),
                _ => {}
            }
        }
        first == Some(self.node.id)
    }

    /// HTML §4.10.5.1: the types `required` applies to.
    fn required_applies(&self) -> bool {
        match self.html_name() {
            Some("select" | "textarea") => true,
            Some("input") => !matches!(
                self.input_type().as_str(),
                "hidden" | "range" | "color" | "submit" | "image" | "reset" | "button"
            ),
            _ => false,
        }
    }

    /// HTML §4.16.3 `:read-write`.
    fn read_write(&self) -> bool {
        let mutable =
            |g: &Self| !g.has(local_name!("readonly")) && g.actually_disabled() != Some(true);
        match self.html_name() {
            Some("textarea") => mutable(self),
            Some("input") => {
                let takes_text = matches!(
                    self.input_type().as_str(),
                    "" | "text"
                        | "search"
                        | "url"
                        | "tel"
                        | "email"
                        | "password"
                        | "date"
                        | "month"
                        | "week"
                        | "time"
                        | "datetime-local"
                        | "number"
                );
                takes_text && mutable(self)
            }
            Some(_) => self.is_editing_host(),
            None => false,
        }
    }

    /// HTML §6.8.1: `contenteditable`, or inside one that isn't false.
    fn is_editing_host(&self) -> bool {
        let mut at = Some(*self);
        while let Some(g) = at {
            match g.node.attr(blitz_dom::LocalName::from("contenteditable")) {
                Some(v) if v.eq_ignore_ascii_case("false") => return false,
                Some(_) => return true,
                None => at = g.up(),
            }
        }
        false
    }

    /// HTML §3.2.6.2 "the language of a node": the nearest `xml:lang` or
    /// `lang`, or none.
    fn language(&self) -> Option<&'a str> {
        let mut at = Some(*self);
        while let Some(g) = at {
            for a in g.node.attrs().into_iter().flatten() {
                let xml = a.name.ns == ns!(xml) && a.name.local.as_ref() == "lang";
                let html = g.html_name().is_some()
                    && a.name.ns.is_empty()
                    && a.name.local.as_ref() == "lang";
                if xml || html {
                    return Some(a.value.as_str());
                }
            }
            at = g.up();
        }
        None
    }
}

/// Selectors 4 §7.2 "extended filtering" (RFC 4647 §3.3.2) of a language
/// tag by a range, ASCII case-insensitive. A `*` as the range's first subtag
/// matches any first subtag; an empty range matches only an empty tag.
fn language_matches(tag: &str, range: &str) -> bool {
    if range.is_empty() {
        return tag.is_empty();
    }
    let mut tags = tag.split('-');
    let mut ranges = range.split('-');
    let (Some(first_tag), Some(first_range)) = (tags.next(), ranges.next()) else {
        return false;
    };
    if first_range != "*" && !first_range.eq_ignore_ascii_case(first_tag) {
        return false;
    }
    if tag.is_empty() {
        return false;
    }
    let mut wanted = ranges.next();
    let mut have = tags.next();
    while let Some(w) = wanted {
        if w == "*" {
            wanted = ranges.next();
            continue;
        }
        let Some(h) = have else {
            return false;
        };
        if w.eq_ignore_ascii_case(h) {
            wanted = ranges.next();
            have = tags.next();
        } else if h.len() == 1 {
            return false;
        } else {
            have = tags.next();
        }
    }
    true
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
        // blitz-dom ignores the case sensitivity the selector asks for.
        self.node.attrs().into_iter().flatten().any(|a| {
            a.name.local == local_name.0
                && match ns {
                    NamespaceConstraint::Any => true,
                    NamespaceConstraint::Specific(n) => a.name.ns == n.0,
                }
                && operation.eval_str(&a.value)
        })
    }

    fn match_non_ts_pseudo_class(
        &self,
        pc: &<Impl as selectors::SelectorImpl>::NonTSPseudoClass,
        context: &mut MatchingContext<'_, Impl>,
    ) -> bool {
        match pc {
            NonTSPseudoClass::Disabled => self.actually_disabled() == Some(true),
            NonTSPseudoClass::Enabled => self.actually_disabled() == Some(false),
            NonTSPseudoClass::Checked => match self.html_name() {
                Some("input") => {
                    matches!(self.input_type().as_str(), "checkbox" | "radio")
                        && self.has(local_name!("checked"))
                }
                Some("option") => self.option_selected(),
                _ => false,
            },
            NonTSPseudoClass::Required => {
                self.required_applies() && self.has(local_name!("required"))
            }
            NonTSPseudoClass::Optional => {
                self.required_applies() && !self.has(local_name!("required"))
            }
            NonTSPseudoClass::ReadWrite => self.read_write(),
            NonTSPseudoClass::ReadOnly => self.html_name().is_some() && !self.read_write(),
            NonTSPseudoClass::Lang(range) => language_matches(self.language().unwrap_or(""), range),
            NonTSPseudoClass::Target => self.budget.target == Some(self.node.id),
            NonTSPseudoClass::Defined => self
                .node
                .element_data()
                .is_some_and(|e| e.name.ns != ns!(html) || !custom_element_name(&e.name.local)),
            _ => Element::match_non_ts_pseudo_class(&self.node, pc, context),
        }
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
        target: Option<usize>,
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
                target,
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
