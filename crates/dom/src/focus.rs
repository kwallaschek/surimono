//! The page's focused element, for `:focus`, `:focus-visible` and
//! `:focus-within` (HTML §4.16.3, Selectors 4 §9).
//!
//! `focus()` and `blur()` run in `glue_html.js`, which fires the events and
//! records the element here through `setFocused`; `document.activeElement`
//! reads it back. Only the page has a navigable, so only the page has focus
//! (HTML §6.6.4). Removing the focused element, or an ancestor of it, leaves
//! nothing focused (HTML §6.6.3 "focus fixup"), so a node that comes back is
//! not focused, as in Chromium 153.

use std::collections::{HashMap, HashSet};

use blitz_dom::ns;
use surimono_webidl::runtime::{BindingResult, ObjectId};

use crate::Document;
use crate::document::Kind;
use crate::select::Focus;

/// Inserted roots kept between two flushes; past this, the next flush looks
/// at the whole page instead, so a script that inserts in a loop holds no
/// more than this.
const MAX_INSERTED: usize = 1024;

/// The page's focus, in its [`Document`].
#[derive(Debug, Default)]
pub(crate) struct FocusState {
    /// The focused element (HTML §6.6.3 "focused area of the document"), if
    /// it isn't the viewport.
    element: Option<usize>,
    /// HTML §6.6.7 "autofocus processed flag".
    autofocus_done: bool,
    /// Candidates are recorded: from the page's lifecycle start until the flag.
    recording: bool,
    /// HTML §6.6.7 "autofocus candidates", in the order they were appended:
    /// the parsed ones, then those found under `inserted` at each flush. Each
    /// entry is a node id with the tag [`track`](FocusState::track) gave it.
    candidates: Vec<(usize, u64)>,
    /// Nodes inserted since the last flush, whose subtrees the next flush looks
    /// at; insertion only notes them.
    inserted: Vec<(usize, u64)>,
    /// More than [`MAX_INSERTED`] were: the next flush looks at the whole page.
    overflow: bool,
    /// The tag of each id in `candidates` and `inserted` whose node still
    /// exists. A dropped node leaves this map, and blitz-dom may give its id
    /// to a new node, which gets a new tag when it is tracked: an entry
    /// counts only while its tag is its id's tag here.
    live: HashMap<usize, u64>,
    /// The next tag.
    next_tag: u64,
    /// Nodes the candidate walks have visited, for tests.
    steps: u64,
    /// The most entries `inserted` has held, for tests.
    inserted_peak: usize,
}

impl FocusState {
    /// The tag of `id`: its current one if it is tracked, else a new one.
    fn track(&mut self, id: usize) -> u64 {
        let next = &mut self.next_tag;
        *self.live.entry(id).or_insert_with(|| {
            *next += 1;
            *next
        })
    }

    /// Whether the entry `(id, tag)` is still the node it was made for.
    fn is_live(&self, (id, tag): (usize, u64)) -> bool {
        self.live.get(&id) == Some(&tag)
    }

    /// `nodes` are inserted (`Document::place_quietly`).
    pub(crate) fn inserting(&mut self, nodes: &[usize]) {
        if !self.recording || self.autofocus_done || self.overflow {
            return;
        }
        if self.inserted.len() + nodes.len() > MAX_INSERTED {
            self.overflow = true;
            self.inserted = Vec::new();
            return;
        }
        for &n in nodes {
            let tag = self.track(n);
            self.inserted.push((n, tag));
        }
        self.inserted_peak = self.inserted_peak.max(self.inserted.len());
    }

    /// Node `id` is dropped (`Document::drop_subtree`).
    pub(crate) fn dropping(&mut self, id: usize) {
        if !self.live.is_empty() {
            self.live.remove(&id);
        }
    }

    /// Tracks the elements a walk found, as candidates.
    fn append(&mut self, found: Vec<usize>) {
        for id in found {
            let tag = self.track(id);
            self.candidates.push((id, tag));
        }
    }
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

    /// The page's lifecycle starts, before any of its scripts runs: the
    /// parser has inserted every element, so those with an `autofocus`
    /// attribute are the first candidates, in tree order (HTML §6.6.7: "When an
    /// element with the autofocus attribute specified is inserted into a
    /// document", step 7 appends it). From now on insertions are noted.
    pub(crate) fn autofocus_parsed(&mut self) -> BindingResult<()> {
        let root = self.root();
        let found = self.autofocus_under(root)?;
        self.focus.append(found);
        self.focus.recording = true;
        Ok(())
    }

    /// The `autofocus` elements under `top`, itself included, in tree order.
    fn autofocus_under(&mut self, top: usize) -> BindingResult<Vec<usize>> {
        let mut found = Vec::new();
        let mut stack = vec![top];
        while let Some(at) = stack.pop() {
            self.poll()?;
            self.focus.steps += 1;
            stack.extend(self.children(at).iter().rev().copied());
            if self.kind(at) == Some(Kind::Element)
                && self.is_html_or_svg(at)
                && self.attribute(at, "autofocus").is_some()
            {
                found.push(at);
            }
        }
        Ok(found)
    }

    /// HTML §6.6.7 "flush autofocus candidates", the part Rust can answer:
    /// the handles of the candidates in the page, newline-joined, for the glue
    /// to focus the first focusable one (steps 5.6 to 5.11); every one of them
    /// leaves the list. Empty when there is nothing to do, at no cost with
    /// nothing inserted since the last flush. Step 4: with candidates but an
    /// element already focused, or a target element, the flag is set.
    ///
    /// Deviation: a node a script inserts is looked at when the flush comes,
    /// not when it is inserted, so an `autofocus` attribute removed (or added)
    /// in between counts as it is at the flush.
    pub(crate) fn autofocus_candidates(&mut self) -> BindingResult<String> {
        if self.focus.autofocus_done || !self.focus.recording {
            return Ok(String::new());
        }
        let inserted = std::mem::take(&mut self.focus.inserted);
        if std::mem::take(&mut self.focus.overflow) {
            let root = self.root();
            let found = self.autofocus_under(root)?;
            self.focus.append(found);
        } else {
            let mut seen = HashSet::new();
            for top in inserted {
                // A root inserted again counts once; a dropped one not at all.
                if !self.focus.is_live(top) || !seen.insert(top.0) {
                    continue;
                }
                let found = self.autofocus_under(top.0)?;
                self.focus.append(found);
            }
        }
        if self.focus.candidates.is_empty() {
            self.focus.live.clear();
            return Ok(String::new());
        }
        let candidates = std::mem::take(&mut self.focus.candidates);
        let live: Vec<bool> = candidates.iter().map(|&c| self.focus.is_live(c)).collect();
        self.focus.live = HashMap::new();
        if self.focus.element.is_some() || self.target_element().is_some() {
            self.done();
            return Ok(String::new());
        }
        // Step 7 of the insertion steps moves one inserted again to the end:
        // the last of its places counts. Only the page's elements (step 5.4).
        let root = self.root();
        let mut seen = HashSet::new();
        let mut keep = Vec::new();
        for (&(id, _), &alive) in candidates.iter().zip(&live).rev() {
            if alive
                && seen.insert(id)
                && self.kind(id) == Some(Kind::Element)
                && self.owner_of_node(id) == root
            {
                keep.push(id);
            }
        }
        let mut out = String::new();
        for &id in keep.iter().rev() {
            let h = self.handle_for(id).map_err(|e| self.fail(e))?;
            out.push_str(&h.to_string());
            out.push('\n');
        }
        Ok(out)
    }

    fn done(&mut self) {
        let f = &mut self.focus;
        f.autofocus_done = true;
        f.candidates = Vec::new();
        f.inserted = Vec::new();
        f.live = HashMap::new();
    }

    /// Nodes the autofocus candidate walks have visited (internal, for tests).
    #[doc(hidden)]
    pub fn autofocus_steps(&self) -> u64 {
        self.focus.steps
    }

    /// The most inserted nodes noted between two flushes (internal, for
    /// tests): at most `MAX_INSERTED`.
    #[doc(hidden)]
    pub fn autofocus_inserted_peak(&self) -> usize {
        self.focus.inserted_peak
    }

    /// Sets the "autofocus processed flag": a candidate is being focused.
    pub(crate) fn autofocus_done(&mut self) -> BindingResult<()> {
        self.done();
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
