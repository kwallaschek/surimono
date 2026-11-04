//! The DOM Standard's tree algorithms (§4.2.3 mutation algorithms, §4.2.6
//! mixin ParentNode, §4.2.8 mixin ChildNode, §4.4 clone and compare, §4.11
//! CharacterData) on top of the document's primitives. Offsets are UTF-16
//! code units, as in the spec.

use blitz_dom::{LocalName, NodeData, QualName, ns};
use html5ever::{Namespace, Prefix};
use selectors::parser::{Component, Selector};
use style::selector_parser::SelectorImpl;
use surimono_webidl::runtime::{BindingError, BindingResult, Value};

use crate::document::{Kind, is_valid_attribute_name, is_valid_element_name};
use crate::select::Matcher;
use crate::{Document, DomError};

/// The XML and XMLNS namespaces, for "validate and extract" (§1.5).
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS: &str = "http://www.w3.org/2000/xmlns/";

/// Deepest bracket nesting accepted in a selector.
const MAX_SELECTOR_NESTING: usize = 32;

/// Longest selector a query takes. The parser can't be stopped, and holds a
/// few dozen bytes per selector in a comma list, in Rust. No real selector
/// is close; a longer one is a `SyntaxError` (a gap, see DESIGN.md).
const MAX_SELECTOR_BYTES: usize = 128 * 1024;

/// Most combinators on any path through a selector, nested ones included:
/// the matcher recurses once per combinator, on the stack. Real selectors
/// have a dozen. Fits the stack with [`MAX_SELECTOR_NESTING`] levels on a
/// 1 MiB thread (`tests/hostile.rs`).
const MAX_SELECTOR_COMBINATORS: usize = 128;

pub(crate) fn dom_error(name: &'static str, message: &str) -> BindingError {
    BindingError::DomException {
        name,
        message: message.to_owned(),
    }
}

fn hierarchy() -> BindingError {
    dom_error(
        "HierarchyRequestError",
        "the node can't be inserted at that position",
    )
}

fn not_found() -> BindingError {
    dom_error("NotFoundError", "the node is not a child of this node")
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl Document {
    fn element_children(&self, id: usize) -> usize {
        self.children(id)
            .iter()
            .filter(|&&c| self.kind(c) == Some(Kind::Element))
            .count()
    }

    /// §4.2.3 "ensure pre-insert validity" and the steps "replace" shares,
    /// in the spec's order: WPT pre-insertion-validation-notfound.js checks
    /// that `child` (the reference child, or the child being replaced) is
    /// tested after the ancestor check and before anything about `node`'s
    /// type. `replacing` is the child being replaced, for the document
    /// checks.
    fn check_insert(
        &mut self,
        parent: usize,
        node: usize,
        child: Option<usize>,
        replacing: Option<usize>,
    ) -> BindingResult<()> {
        let parent_kind = self.kind(parent);
        if !matches!(
            parent_kind,
            Some(Kind::Document | Kind::DocumentFragment | Kind::Element)
        ) || self.is_inclusive_ancestor(node, parent)
        {
            return Err(hierarchy());
        }
        if child.is_some_and(|c| self.parent(c) != Some(parent)) {
            return Err(not_found());
        }
        let kind = self.kind(node);
        if !matches!(
            kind,
            Some(Kind::DocumentFragment | Kind::Element | Kind::Text | Kind::Comment)
        ) {
            return Err(hierarchy());
        }
        if parent_kind == Some(Kind::Document) {
            // Another element than the one being replaced.
            let other_element = self
                .children(parent)
                .iter()
                .any(|&c| self.kind(c) == Some(Kind::Element) && Some(c) != replacing);
            let bad = match kind {
                Some(Kind::Text) => true,
                Some(Kind::Element) => other_element,
                Some(Kind::DocumentFragment) => {
                    let elements = self.element_children(node);
                    let text = self
                        .children(node)
                        .iter()
                        .any(|&c| self.kind(c) == Some(Kind::Text));
                    text || elements > 1 || (elements == 1 && other_element)
                }
                _ => false,
            };
            if bad {
                return Err(hierarchy());
            }
        }
        let height = if kind == Some(Kind::DocumentFragment) {
            self.height(node) - 1
        } else {
            self.height(node)
        };
        self.check_depth(parent, height).map_err(|e| self.fail(e))
    }

    /// §4.2.3 "insert", after validity: `node`, or a fragment's children,
    /// before `before`; returns what went in. A fragment queues its own
    /// record, and a node leaving another parent its, whatever `quiet` (the
    /// "suppress observers" flag) says: that only covers the insertion.
    fn insert_checked(
        &mut self,
        parent: usize,
        node: usize,
        before: Option<usize>,
        quiet: bool,
    ) -> Vec<usize> {
        let nodes = if self.kind(node) == Some(Kind::DocumentFragment) {
            if self.is_observed() && !self.children(node).is_empty() {
                let children = self.children(node).to_vec();
                self.note_child_list(node, &[], &children, None, None);
            }
            let children = self.take_children(node);
            for &c in &children {
                self.set_parent(c, None);
            }
            children
        } else {
            self.detach(node);
            vec![node]
        };
        if quiet {
            self.place_quietly(parent, &nodes, before);
        } else {
            self.place(parent, &nodes, before);
        }
        nodes
    }

    /// §4.2.3 "pre-insert".
    pub(crate) fn pre_insert(
        &mut self,
        parent: usize,
        node: usize,
        child: Option<usize>,
    ) -> BindingResult<()> {
        self.check_insert(parent, node, child, None)?;
        // "If referenceChild is node, set it to node's next sibling."
        let before = if child == Some(node) {
            self.sibling(node, true)
        } else {
            child
        };
        self.insert_checked(parent, node, before, false);
        Ok(())
    }

    /// §4.2.3 "replace": one record, with both the added and removed nodes.
    pub(crate) fn replace(
        &mut self,
        parent: usize,
        node: usize,
        child: usize,
    ) -> BindingResult<()> {
        self.check_insert(parent, node, Some(child), Some(child))?;
        let mut before = self.sibling(child, true);
        if before == Some(node) {
            before = self.sibling(node, true);
        }
        let previous = self.sibling(child, false);
        // "Adopt" comes first and isn't suppressed. It takes `child` itself
        // away when the two are one, which leaves no one to remove.
        self.detach(node);
        let removed = if self.parent(child) == Some(parent) {
            self.detach_quietly(child);
            Some(child)
        } else {
            None
        };
        let added = self.insert_checked(parent, node, before, true);
        if self.is_observed() {
            let removed = removed.as_slice();
            self.note_child_list(parent, &added, removed, previous, before);
        }
        Ok(())
    }

    /// §4.2.3 "replace all": `parent`'s children go, `node` (which fits, and
    /// may be a fragment) comes in, and one record tells both.
    pub(crate) fn replace_all(&mut self, parent: usize, node: Option<usize>) {
        let removed = if self.is_observed() {
            let children = self.children(parent).to_vec();
            // The record, queued last, must find them alive.
            for &c in &children {
                if let Err(e) = self.handle_for(c) {
                    self.fault.get_or_insert(e);
                }
            }
            children
        } else {
            Vec::new()
        };
        self.remove_children_quietly(parent);
        let added = node.map_or_else(Vec::new, |n| self.insert_checked(parent, n, None, true));
        if self.is_observed() && !(added.is_empty() && removed.is_empty()) {
            self.note_child_list(parent, &added, &removed, None, None);
        }
    }

    /// §4.2.3 "pre-remove".
    pub(crate) fn pre_remove(&mut self, parent: usize, child: usize) -> BindingResult<()> {
        if self.parent(child) != Some(parent) {
            return Err(not_found());
        }
        self.detach(child);
        Ok(())
    }

    /// Puts a parsed fragment's children under `parent`, before `before`,
    /// and frees the fragment. The parse checked the limits.
    pub(crate) fn insert_parsed(&mut self, parent: usize, frag: usize, before: Option<usize>) {
        self.insert_checked(parent, frag, before, false);
        self.drop_subtree(frag);
    }

    /// `innerHTML` set: a parsed fragment replaces `parent`'s children.
    pub(crate) fn replace_all_parsed(&mut self, parent: usize, frag: usize) {
        self.replace_all(parent, Some(frag));
        self.drop_subtree(frag);
    }

    /// `outerHTML` set: a parsed fragment replaces `child`.
    pub(crate) fn replace_with_parsed(
        &mut self,
        parent: usize,
        frag: usize,
        child: usize,
    ) -> BindingResult<()> {
        let r = self.replace(parent, frag, child);
        self.drop_subtree(frag);
        r
    }

    /// Selectors nest in brackets; the parser and the matcher recurse per
    /// level. Refused here so that no script can reach the stack's end.
    /// Brackets in strings count, on purpose: telling them apart needs the
    /// CSS tokenizer's every rule (a `"` in an unquoted `url(` starts no
    /// string, and `:is()` recovers from that token), and a wrong guess here
    /// would let real nesting through. See "Deviations and gaps" in DESIGN.md.
    fn check_selector_text(selectors: &str) -> BindingResult<()> {
        if selectors.len() > MAX_SELECTOR_BYTES {
            return Err(dom_error("SyntaxError", "the selector is too long"));
        }
        let mut depth = 0usize;
        for b in selectors.bytes() {
            match b {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
            if depth > MAX_SELECTOR_NESTING {
                return Err(dom_error("SyntaxError", "the selector nests too deeply"));
            }
        }
        Ok(())
    }

    fn parse_selectors(
        &self,
        selectors: &str,
    ) -> BindingResult<selectors::SelectorList<SelectorImpl>> {
        Self::check_selector_text(selectors)?;
        let list = self
            .selector_list(selectors)
            .ok_or_else(|| dom_error("SyntaxError", "the selector is not valid"))?;
        if combinator_depth(list.slice()) > MAX_SELECTOR_COMBINATORS {
            return Err(dom_error("SyntaxError", "the selector is too complex"));
        }
        Ok(list)
    }

    fn matcher(&self) -> Matcher {
        Matcher::new(self.quirks_mode(), self.deadline.clone())
    }

    /// `querySelector(All)` (DOM §4.2.6): descendants of `top`, in tree
    /// order, matching the selectors against the whole tree. See
    /// [`crate::select`] for why this isn't blitz-dom's query.
    pub(crate) fn select(
        &mut self,
        top: usize,
        selectors: &str,
        first_only: bool,
    ) -> BindingResult<Vec<usize>> {
        let list = self.parse_selectors(selectors)?;
        let mut matcher = self.matcher();
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self.children(top).iter().rev().copied().collect();
        // A document has no scope element; a fragment should be one, a gap
        // in DESIGN.md.
        let scope = (self.kind(top) == Some(Kind::Element)).then(|| self.node(top));
        while let Some(n) = stack.pop() {
            if self.kind(n) == Some(Kind::Element) && matcher.matches(self.node(n), scope, &list) {
                out.push(n);
                if first_only {
                    break;
                }
            }
            if matcher.stopped() {
                return Err(timed_out());
            }
            stack.extend(self.children(n).iter().rev().copied());
        }
        Ok(out)
    }

    pub(crate) fn element_matches(&mut self, id: usize, selectors: &str) -> BindingResult<bool> {
        let list = self.parse_selectors(selectors)?;
        let mut matcher = self.matcher();
        let node = self.node(id);
        let found = matcher.matches(node, Some(node), &list);
        if matcher.stopped() {
            return Err(timed_out());
        }
        Ok(found)
    }

    /// `closest`: `id` or its nearest matching ancestor element.
    pub(crate) fn closest_match(
        &mut self,
        id: usize,
        selectors: &str,
    ) -> BindingResult<Option<usize>> {
        let list = self.parse_selectors(selectors)?;
        let mut matcher = self.matcher();
        let mut at = Some(id);
        while let Some(n) = at {
            if self.kind(n) == Some(Kind::Element)
                && matcher.matches(self.node(n), Some(self.node(id)), &list)
            {
                return Ok(Some(n));
            }
            if matcher.stopped() {
                return Err(timed_out());
            }
            at = self.parent(n);
        }
        Ok(None)
    }

    /// §4.2.6 "convert nodes into a node": `values`' nodes, with strings as
    /// new text nodes, as one node (a new fragment if not exactly one).
    /// Returns the node, and whether it is a temporary fragment.
    fn nodes_into_node(&mut self, values: Vec<Value>) -> BindingResult<(usize, bool)> {
        let mut nodes = Vec::with_capacity(values.len());
        for v in values {
            nodes.push(match v {
                Value::Object(id) => self.node_of(id)?,
                Value::String(s) => self.create_text(&s).map_err(|e| self.fail(e))?,
                _ => return Err(BindingError::TypeError("expected a node or string".into())),
            });
        }
        if let [one] = nodes.as_slice() {
            return Ok((*one, false));
        }
        let frag = self.create_fragment().map_err(|e| self.fail(e))?;
        for n in nodes {
            self.pre_insert(frag, n, None)?;
        }
        Ok((frag, true))
    }

    /// Frees a temporary fragment from [`nodes_into_node`], so a loop of
    /// `append(a, b)` doesn't fill the document with them. After a refused
    /// insertion it still has its children; those a script holds stay alive.
    fn finish_nodes(&mut self, node: usize, temporary: bool) {
        if temporary {
            self.drop_subtree(node);
        }
    }

    /// The `Object` ids in `values`, as nodes.
    fn node_ids(&mut self, values: &[Value]) -> BindingResult<Vec<usize>> {
        let mut out = Vec::new();
        for v in values {
            if let Value::Object(id) = v {
                out.push(self.node_of(*id)?);
            }
        }
        Ok(out)
    }

    pub(crate) fn prepend_nodes(&mut self, parent: usize, nodes: Vec<Value>) -> BindingResult<()> {
        let (node, temporary) = self.nodes_into_node(nodes)?;
        let first = self.children(parent).first().copied();
        let r = self.pre_insert(parent, node, first);
        self.finish_nodes(node, temporary);
        r
    }

    pub(crate) fn append_nodes(&mut self, parent: usize, nodes: Vec<Value>) -> BindingResult<()> {
        let (node, temporary) = self.nodes_into_node(nodes)?;
        let r = self.pre_insert(parent, node, None);
        self.finish_nodes(node, temporary);
        r
    }

    pub(crate) fn replace_children_with(
        &mut self,
        parent: usize,
        nodes: Vec<Value>,
    ) -> BindingResult<()> {
        let (node, temporary) = self.nodes_into_node(nodes)?;
        let r = self.check_insert(parent, node, None, None).or_else(|e| {
            // "replace all" drops the old children first, so a document
            // may swap its element; only a real problem remains.
            if self.kind(parent) == Some(Kind::Document) {
                let old = self
                    .children(parent)
                    .iter()
                    .copied()
                    .find(|&c| self.kind(c) == Some(Kind::Element));
                self.check_insert(parent, node, None, old)
            } else {
                Err(e)
            }
        });
        if r.is_ok() {
            self.replace_all(parent, Some(node));
        }
        self.finish_nodes(node, temporary);
        r
    }

    /// §4.2.8 `before`.
    pub(crate) fn before_nodes(&mut self, this: usize, nodes: Vec<Value>) -> BindingResult<()> {
        let Some(parent) = self.parent(this) else {
            return Ok(());
        };
        let listed = self.node_ids(&nodes)?;
        let mut viable = self.sibling(this, false);
        while let Some(s) = viable.filter(|s| listed.contains(s)) {
            viable = self.sibling(s, false);
        }
        let (node, temporary) = self.nodes_into_node(nodes)?;
        let before = match viable {
            None => self.children(parent).first().copied(),
            Some(v) => self.sibling(v, true),
        };
        let r = self.pre_insert(parent, node, before);
        self.finish_nodes(node, temporary);
        r
    }

    fn viable_next(&self, this: usize, listed: &[usize]) -> Option<usize> {
        let mut viable = self.sibling(this, true);
        while let Some(s) = viable.filter(|s| listed.contains(s)) {
            viable = self.sibling(s, true);
        }
        viable
    }

    /// §4.2.8 `after`.
    pub(crate) fn after_nodes(&mut self, this: usize, nodes: Vec<Value>) -> BindingResult<()> {
        let Some(parent) = self.parent(this) else {
            return Ok(());
        };
        let listed = self.node_ids(&nodes)?;
        let next = self.viable_next(this, &listed);
        let (node, temporary) = self.nodes_into_node(nodes)?;
        let r = self.pre_insert(parent, node, next);
        self.finish_nodes(node, temporary);
        r
    }

    /// §4.2.8 `replaceWith`.
    pub(crate) fn replace_with_nodes(
        &mut self,
        this: usize,
        nodes: Vec<Value>,
    ) -> BindingResult<()> {
        let Some(parent) = self.parent(this) else {
            return Ok(());
        };
        let listed = self.node_ids(&nodes)?;
        let next = self.viable_next(this, &listed);
        let (node, temporary) = self.nodes_into_node(nodes)?;
        let r = if self.parent(this) == Some(parent) {
            self.replace(parent, node, this)
        } else {
            self.pre_insert(parent, node, next)
        };
        self.finish_nodes(node, temporary);
        r
    }

    /// §4.4 `cloneNode`. Iterative, and refuses up front if the copy
    /// wouldn't fit. A document can't be cloned yet: a runtime holds one
    /// (see "Deviations and gaps" in DESIGN.md), so `clone_one` ends the
    /// script with [`DomError::Unsupported`], never a DOM exception.
    pub(crate) fn clone_tree(&mut self, top: usize, deep: bool) -> BindingResult<usize> {
        let copy = self.clone_one(top)?;
        if deep {
            let mut size = 0;
            let mut stack = vec![top];
            while let Some(n) = stack.pop() {
                let c = self.children(n);
                size += c.len();
                stack.extend_from_slice(c);
                stack.extend(self.template_of(n));
            }
            if let Err(e) = self.check_room_for(size) {
                self.drop_subtree(copy);
                return Err(self.fail(e));
            }
            let mut stack: Vec<(usize, usize)> = vec![(top, copy)];
            while let Some((from, to)) = stack.pop() {
                // HTML §4.12.3 cloning steps: a template's content too.
                if let Some(content) = self.template_of(from) {
                    let new_content = self.template_content(to).map_err(|e| self.fail(e))?;
                    stack.push((content, new_content));
                }
                let children: Vec<usize> = self.children(from).to_vec();
                let mut made = Vec::with_capacity(children.len());
                for c in children {
                    let n = self.clone_one(c)?;
                    made.push(n);
                    stack.push((c, n));
                }
                self.place(to, &made, None);
            }
        }
        Ok(copy)
    }

    fn clone_one(&mut self, id: usize) -> BindingResult<usize> {
        let made = match &self.node(id).data {
            NodeData::Text(_) => {
                let text = self.char_data(id).to_owned();
                self.create_text(&text)
            }
            NodeData::Comment => {
                let data = self.char_data(id).to_owned();
                self.create_comment(&data)
            }
            NodeData::Element(e) | NodeData::AnonymousBlock(e) => {
                let name = e.name.clone();
                let attrs = e.attrs().to_vec();
                // The copy's `style` goes through stylo, in one call: every
                // attribute passed `style_attr::fits` on its way in, so this
                // is bounded, but it counts for more than one step.
                let style = e.attr(blitz_dom::local_name!("style"));
                self.poll_by(style.map_or(0, str::len))?;
                self.create_element_with(name, attrs)
            }
            NodeData::Document => {
                return Err(self.fail(DomError::Unsupported {
                    feature: "cloning a document",
                }));
            }
        };
        made.map_err(|e| self.fail(e))
    }

    /// §4.4 `isEqualNode`, iteratively.
    pub(crate) fn nodes_equal(&self, a: usize, b: usize) -> bool {
        let mut stack = vec![(a, b)];
        while let Some((x, y)) = stack.pop() {
            let (nx, ny) = (self.node(x), self.node(y));
            let same = match (&nx.data, &ny.data) {
                (NodeData::Text(_), NodeData::Text(_)) | (NodeData::Comment, NodeData::Comment) => {
                    self.char_data(x) == self.char_data(y)
                }
                (NodeData::Element(p), NodeData::Element(q)) => {
                    p.name == q.name
                        && p.attrs().len() == q.attrs().len()
                        && p.attrs().iter().all(|a| {
                            q.attrs().iter().any(|o| {
                                same_attribute_name(&o.name, &a.name) && o.value == a.value
                            })
                        })
                }
                (NodeData::Document, NodeData::Document) => true,
                _ => false,
            };
            if !same || nx.children.len() != ny.children.len() {
                return false;
            }
            stack.extend(nx.children.iter().copied().zip(ny.children.iter().copied()));
        }
        true
    }

    /// `id` and its ancestors, root first.
    fn path(&self, id: usize) -> Vec<usize> {
        let mut out = vec![id];
        let mut at = id;
        while let Some(p) = self.parent(at) {
            out.push(p);
            at = p;
        }
        out.reverse();
        out
    }

    /// §4.4 `compareDocumentPosition`: the position of `other` relative to
    /// `this`.
    pub(crate) fn compare_position(&self, this: usize, other: usize) -> u16 {
        const DISCONNECTED: u16 = 1;
        const PRECEDING: u16 = 2;
        const FOLLOWING: u16 = 4;
        const CONTAINS: u16 = 8;
        const CONTAINED_BY: u16 = 16;
        const IMPLEMENTATION_SPECIFIC: u16 = 32;
        if this == other {
            return 0;
        }
        let (p, q) = (self.path(this), self.path(other));
        if p[0] != q[0] {
            // "Consistent" between the two calls: by node number.
            let order = if other < this { PRECEDING } else { FOLLOWING };
            return DISCONNECTED | IMPLEMENTATION_SPECIFIC | order;
        }
        let shared = p.iter().zip(&q).take_while(|(a, b)| a == b).count();
        if shared == p.len() {
            return CONTAINED_BY | FOLLOWING;
        }
        if shared == q.len() {
            return CONTAINS | PRECEDING;
        }
        let parent = p[shared - 1];
        let list = self.children(parent);
        let at = |n| list.iter().position(|&c| c == n);
        if at(q[shared]) < at(p[shared]) {
            PRECEDING
        } else {
            FOLLOWING
        }
    }

    /// §4.4 `normalize`: below `top`, no empty text nodes and no adjacent
    /// text nodes. Linear in the subtree: each parent's list is rebuilt in
    /// one pass, where removing nodes one at a time would rescan it. Merged
    /// nodes go before the joined text is written, so a merge never needs
    /// room for both copies.
    pub(crate) fn normalize_tree(&mut self, top: usize) -> BindingResult<()> {
        if self.is_observed() {
            return self.normalize_observed(top);
        }
        let mut stack = vec![top];
        while let Some(n) = stack.pop() {
            let mut kept = Vec::with_capacity(self.children(n).len());
            let mut merged = Vec::new();
            // Each run's first node, with the text of the nodes merged in.
            let mut runs: Vec<(usize, String)> = Vec::new();
            let mut in_run = false;
            for &c in self.children(n) {
                self.poll()?;
                if self.kind(c) != Some(Kind::Text) {
                    in_run = false;
                    kept.push(c);
                    stack.push(c);
                    continue;
                }
                let text = self.char_data(c);
                match runs.last_mut().filter(|_| in_run) {
                    Some((_, joined)) => {
                        joined.push_str(text);
                        merged.push(c);
                    }
                    None if text.is_empty() => merged.push(c),
                    None => {
                        in_run = true;
                        runs.push((c, String::new()));
                        kept.push(c);
                    }
                }
            }
            if !merged.is_empty() {
                self.replace_child_list(n, kept, merged);
            }
            for (id, joined) in runs {
                self.poll()?;
                if !joined.is_empty() {
                    let mut text = self.char_data(id).to_owned();
                    text.push_str(&joined);
                    self.set_text_data(id, &text).map_err(|e| self.fail(e))?;
                }
            }
        }
        Ok(())
    }

    /// `normalize` with an observer about. Every step queues its own record,
    /// in §4.4's order: for each text node in tree order, the joined text
    /// (a record even if nothing joined it), then one removal for each node
    /// merged into it. So nodes leave one at
    /// a time and need the room for both copies, and a wide list is quadratic,
    /// which each removal pays for in its poll.
    fn normalize_observed(&mut self, top: usize) -> BindingResult<()> {
        let mut stack: Vec<usize> = self.children(top).iter().rev().copied().collect();
        while let Some(n) = stack.pop() {
            self.poll()?;
            if self.kind(n) != Some(Kind::Text) {
                stack.extend(self.children(n).iter().rev().copied());
                continue;
            }
            if self.char_data(n).is_empty() {
                self.remove_observed(n)?;
                continue;
            }
            let mut merged = Vec::new();
            let mut joined = String::new();
            let mut next = self.sibling(n, true);
            while let Some(s) = next.filter(|&s| self.kind(s) == Some(Kind::Text)) {
                self.poll()?;
                joined.push_str(self.char_data(s));
                merged.push(s);
                next = self.sibling(s, true);
            }
            // Step 4 replaces data even when `joined` is empty, and "replace
            // data" always queues its record.
            let mut text = self.char_data(n).to_owned();
            text.push_str(&joined);
            self.set_text_data(n, &text).map_err(|e| self.fail(e))?;
            // They are next on the stack, and may not outlive their removal.
            stack.truncate(stack.len().saturating_sub(merged.len()));
            for s in merged {
                self.remove_observed(s)?;
            }
        }
        Ok(())
    }

    /// Removes `id` with its record, freeing it if nothing can reach it.
    fn remove_observed(&mut self, id: usize) -> BindingResult<()> {
        let Some(parent) = self.parent(id) else {
            return Ok(());
        };
        self.poll_by(self.children(parent).len())?;
        let previous = self.sibling(id, false);
        let next = self.sibling(id, true);
        self.note_child_list(parent, &[], &[id], previous, next);
        let kept = self.children(parent).iter().copied();
        let kept = kept.filter(|&c| c != id).collect();
        self.replace_child_list(parent, kept, vec![id]);
        Ok(())
    }

    fn character_data(&self, id: usize) -> Vec<u16> {
        utf16(&self.text_content(id).unwrap_or_default())
    }

    /// §4.11 "replace data". An offset inside a surrogate pair leaves half a
    /// pair, which UTF-8 storage can't hold; it becomes U+FFFD (a gap in
    /// DESIGN.md).
    pub(crate) fn replace_data(
        &mut self,
        id: usize,
        offset: u32,
        count: u32,
        data: &str,
    ) -> BindingResult<()> {
        let units = self.character_data(id);
        let offset = offset as usize;
        if offset > units.len() {
            return Err(index_size());
        }
        let end = offset.saturating_add(count as usize).min(units.len());
        let mut out = String::from_utf16_lossy(&units[..offset]);
        out.push_str(data);
        out.push_str(&String::from_utf16_lossy(&units[end..]));
        self.set_text_content(id, &out).map_err(|e| self.fail(e))
    }

    /// §4.11 "substring data".
    pub(crate) fn substring_data(
        &self,
        id: usize,
        offset: u32,
        count: u32,
    ) -> BindingResult<String> {
        let units = self.character_data(id);
        let offset = offset as usize;
        if offset > units.len() {
            return Err(index_size());
        }
        let end = offset.saturating_add(count as usize).min(units.len());
        Ok(String::from_utf16_lossy(&units[offset..end]))
    }

    /// §4.12 "split".
    pub(crate) fn split_text(&mut self, id: usize, offset: u32) -> BindingResult<usize> {
        let units = self.character_data(id);
        let at = offset as usize;
        if at > units.len() {
            return Err(index_size());
        }
        let tail = String::from_utf16_lossy(&units[at..]);
        let new = self.create_text(&tail).map_err(|e| self.fail(e))?;
        if let Some(parent) = self.parent(id) {
            let next = self.sibling(id, true);
            self.pre_insert(parent, new, next)?;
        }
        let head = String::from_utf16_lossy(&units[..at]);
        self.set_text_data(id, &head).map_err(|e| self.fail(e))?;
        Ok(new)
    }

    /// §4.12 `wholeText`: the data of the contiguous text nodes around `id`.
    pub(crate) fn whole_text(&self, id: usize) -> String {
        let mut first = id;
        while let Some(p) = self
            .sibling(first, false)
            .filter(|&s| self.kind(s) == Some(Kind::Text))
        {
            first = p;
        }
        let mut out = self.text_content(first).unwrap_or_default();
        let mut at = first;
        while let Some(n) = self
            .sibling(at, true)
            .filter(|&s| self.kind(s) == Some(Kind::Text))
        {
            out.push_str(&self.text_content(n).unwrap_or_default());
            at = n;
        }
        out
    }

    /// §1.5 "validate and extract": the name for `createElementNS` and
    /// `setAttributeNS`.
    pub(crate) fn validate_and_extract(
        namespace: Option<&str>,
        qualified: &str,
        element: bool,
    ) -> BindingResult<QualName> {
        let namespace = namespace.filter(|n| !n.is_empty());
        let (prefix, local) = match qualified.split_once(':') {
            Some((p, l)) => (Some(p), l),
            None => (None, qualified),
        };
        let prefix_ok = prefix.is_none_or(|p| {
            !p.is_empty() && !p.contains(['\t', '\n', '\u{c}', '\r', ' ', '\0', '/', '>'])
        });
        let local_ok = if element {
            is_valid_element_name(local)
        } else {
            is_valid_attribute_name(local)
        };
        if !prefix_ok || !local_ok {
            return Err(dom_error(
                "InvalidCharacterError",
                "the qualified name is not valid",
            ));
        }
        let namespace_error = || dom_error("NamespaceError", "prefix and namespace don't match");
        if prefix.is_some() && namespace.is_none() {
            return Err(namespace_error());
        }
        if prefix == Some("xml") && namespace != Some(XML) {
            return Err(namespace_error());
        }
        let xmlns_name = qualified == "xmlns" || prefix == Some("xmlns");
        if xmlns_name != (namespace == Some(XMLNS)) {
            return Err(namespace_error());
        }
        Ok(QualName::new(
            prefix.map(Prefix::from),
            namespace.map_or(ns!(), Namespace::from),
            LocalName::from(local),
        ))
    }

    /// `insertAdjacentElement`/`Text` (§4.9 "insert adjacent"): the node to
    /// insert `node` under and before, or `None` for no parent.
    pub(crate) fn insert_adjacent(
        &mut self,
        this: usize,
        place: &str,
        node: usize,
    ) -> BindingResult<bool> {
        match place.to_ascii_lowercase().as_str() {
            "beforebegin" => {
                let Some(parent) = self.parent(this) else {
                    return Ok(false);
                };
                self.pre_insert(parent, node, Some(this))?;
            }
            "afterbegin" => {
                let first = self.children(this).first().copied();
                self.pre_insert(this, node, first)?;
            }
            "beforeend" => self.pre_insert(this, node, None)?,
            "afterend" => {
                let Some(parent) = self.parent(this) else {
                    return Ok(false);
                };
                let next = self.sibling(this, true);
                self.pre_insert(parent, node, next)?;
            }
            _ => {
                return Err(dom_error(
                    "SyntaxError",
                    "the position must be beforebegin, afterbegin, beforeend or afterend",
                ));
            }
        }
        Ok(true)
    }
}

/// The most combinators on one path through `list`: a selector's own, plus
/// the worst list nested in it. Recursion is bounded by
/// [`MAX_SELECTOR_NESTING`], checked first.
fn combinator_depth(list: &[Selector<SelectorImpl>]) -> usize {
    list.iter().map(selector_combinators).max().unwrap_or(0)
}

fn selector_combinators(selector: &Selector<SelectorImpl>) -> usize {
    let mut own = 0;
    let mut nested = 0;
    for c in selector.iter_raw_match_order() {
        let inner = match c {
            Component::Combinator(_) => {
                own += 1;
                0
            }
            Component::Negation(l) | Component::Is(l) | Component::Where(l) => {
                combinator_depth(l.slice())
            }
            Component::NthOf(d) => combinator_depth(d.selectors()),
            Component::Has(r) => r
                .iter()
                .map(|s| selector_combinators(&s.selector))
                .max()
                .unwrap_or(0),
            Component::Slotted(s) | Component::Host(Some(s)) => selector_combinators(s),
            _ => 0,
        };
        nested = usize::max(nested, inner);
    }
    own + nested
}

/// The sandbox stops the script at its next check, so this is what the
/// script never gets to see: the call fails with `TimeLimit`.
pub(crate) fn timed_out() -> BindingError {
    BindingError::TypeError("the time limit has passed".to_owned())
}

/// §4.4 "equals" for attributes: namespace and local name, with the value
/// compared by the caller. An attribute's prefix doesn't count (an element's
/// does); WPT Node-isEqualNode.html.
fn same_attribute_name(a: &QualName, b: &QualName) -> bool {
    a.ns == b.ns && a.local == b.local
}

fn index_size() -> BindingError {
    dom_error("IndexSizeError", "the offset is past the end of the data")
}
