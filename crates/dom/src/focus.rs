//! The page's focused element, for `:focus`, `:focus-visible` and
//! `:focus-within` (HTML §4.16.3, Selectors 4 §9).
//!
//! `focus()` and `blur()` run in `glue_html.js`, which fires the events and
//! records the element here through `setFocused`; `document.activeElement`
//! reads it back. Only the page has a navigable, so only the page has focus
//! (HTML §6.6.4). Removing the focused element, or an ancestor of it, leaves
//! nothing focused (HTML §6.6.3 "focus fixup"), so a node that comes back is
//! not focused, as in Chromium 153.

use blitz_dom::ns;
use surimono_webidl::runtime::{BindingResult, ObjectId};

use crate::Document;
use crate::document::Kind;
use crate::select::Focus;

/// The page's focus, in its [`Document`].
#[derive(Debug, Default)]
pub(crate) struct FocusState {
    /// The focused element (HTML §6.6.3 "focused area of the document"), if
    /// it isn't the viewport.
    element: Option<usize>,
    /// HTML §6.6.5 "autofocus processed flag".
    autofocus_done: bool,
    /// The tree's `version` when the autofocus candidates were last looked for.
    autofocus_scanned: Option<u64>,
}

impl Document {
    /// Records `this`, an element of the page, as focused, or nothing for a
    /// document.
    pub(crate) fn set_focused(&mut self, this: ObjectId) -> BindingResult<()> {
        let id = self.node_of(this)?;
        self.focus.element = (self.kind(id) == Some(Kind::Element)
            && self.owner_of_node(id) == self.root()
            && self.is_connected(id))
        .then_some(id);
        Ok(())
    }

    /// The handle of the page's focused element, 0 for none.
    pub(crate) fn focused_element(&mut self) -> BindingResult<u32> {
        match self.focus.element {
            Some(id) => self.handle_for(id).map_err(|e| self.fail(e)),
            None => Ok(0),
        }
    }

    /// The focused element, for one query in `doc`: none outside the page.
    pub(crate) fn focused_in(&self, doc: usize) -> Option<usize> {
        self.focus.element.filter(|_| doc == self.root())
    }

    /// `removed` is about to leave its parent: the focused element goes if
    /// it is `removed` or under it.
    pub(crate) fn focus_removing(&mut self, removed: usize) {
        let Some(mut at) = self.focus.element else {
            return;
        };
        loop {
            if at == removed {
                self.focus.element = None;
                return;
            }
            match self.node(at).parent {
                Some(p) => at = p,
                None => return,
            }
        }
    }

    /// The children `gone` of `parent` are about to leave it together.
    pub(crate) fn focus_removing_children(&mut self, parent: usize, gone: &[usize]) {
        let Some(mut at) = self.focus.element else {
            return;
        };
        // The focused element's ancestor (or itself) that is `parent`'s child.
        while let Some(p) = self.node(at).parent {
            if p == parent {
                if gone.contains(&at) {
                    self.focus.element = None;
                }
                return;
            }
            at = p;
        }
    }

    /// HTML §6.6.5 "flush autofocus candidates", the part Rust can answer:
    /// the handles of the page's elements with an `autofocus` attribute, in
    /// tree order and newline-joined, for the glue to focus the first
    /// focusable one; empty when there is nothing to do. The standard keeps
    /// the elements in the order they were inserted; here they are looked for
    /// again whenever the tree changed since the last look, so an element
    /// whose attribute is removed after it was inserted is no candidate, and
    /// one that wasn't focusable then is tried again. With candidates but an
    /// element already focused, or a target element (`:target`), autofocus is
    /// over, as the standard's step 4 says.
    pub(crate) fn autofocus_candidates(&mut self) -> BindingResult<String> {
        if self.focus.autofocus_done || self.focus.autofocus_scanned == Some(self.version) {
            return Ok(String::new());
        }
        self.focus.autofocus_scanned = Some(self.version);
        let mut found = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(at) = stack.pop() {
            self.poll()?;
            stack.extend(self.children(at).iter().rev().copied());
            if self.kind(at) == Some(Kind::Element)
                && self.is_html_or_svg(at)
                && self.attribute(at, "autofocus").is_some()
            {
                found.push(at);
            }
        }
        if found.is_empty() {
            return Ok(String::new());
        }
        if self.focus.element.is_some() || self.target_element().is_some() {
            self.focus.autofocus_done = true;
            return Ok(String::new());
        }
        let mut out = String::new();
        for id in found {
            let h = self.handle_for(id).map_err(|e| self.fail(e))?;
            out.push_str(&h.to_string());
            out.push('\n');
        }
        Ok(out)
    }

    /// Sets the "autofocus processed flag": a candidate is being focused.
    pub(crate) fn autofocus_done(&mut self) -> BindingResult<()> {
        self.focus.autofocus_done = true;
        Ok(())
    }

    /// An element in the HTML or SVG namespace, which `autofocus` is
    /// defined on (`HTMLOrSVGElement`).
    fn is_html_or_svg(&self, id: usize) -> bool {
        self.element_name(id)
            .is_some_and(|n| n.ns == ns!(html) || n.ns == ns!(svg))
    }

    /// What a selector query in `doc` matches the focus pseudo-classes with.
    pub(crate) fn focus_for_query(&self, doc: usize) -> Focus {
        let Some(element) = self.focused_in(doc) else {
            return Focus::default();
        };
        let mut within = vec![element];
        let mut at = element;
        while let Some(p) = self.node(at).parent {
            // The document node is no element; nothing matches it.
            if self.kind(p) == Some(Kind::Element) {
                within.push(p);
            }
            at = p;
        }
        within.sort_unstable();
        Focus {
            element: Some(element),
            within,
        }
    }
}
