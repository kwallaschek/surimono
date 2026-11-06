//! DOM §5 ranges, and what mutations do to live ones.
//!
//! A range is two boundary points in Rust, because Rust sees the mutations. The
//! script holds a handle to `Object::Range`, an index into
//! `Document::ranges`, and every node a boundary point names has a handle, so
//! it is never dropped (or its slab slot reused) under the range.
//!
//! **What a mutation pays.** A page with no ranges pays one `is_empty`. With
//! ranges, `range_points` counts boundary points by node, so an insertion or a
//! data change on a node no range touches is one lookup, and a removal looks at
//! the removed subtree only up to about the size of the range list. Past that,
//! or when a range is touched, it walks every range: at most `max_ranges` of
//! them, each up its ancestors, under [`Document::poll`]. So a hostile page gets
//! `RangeLimit` from too many ranges, and `TimeLimit` from costly mutations
//! whatever the ranges cost, in the native that is running. Past the deadline a
//! walk stops where it is; the script is stopped at its next check, and nothing
//! here relies on a boundary being right, only on its node being alive.

use std::cmp::Ordering;
use std::collections::HashMap;

use blitz_dom::ns;
use surimono_webidl::runtime::{BindingError, BindingResult, ObjectId, Value};
use surimono_webidl::stubs as idl;

use crate::document::{Handle, Kind, Object};
use crate::tree::dom_error;
use crate::{Document, DomError};

/// A boundary point: a node and an offset in UTF-16 code units or children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Point {
    pub node: usize,
    pub offset: u32,
}

/// A live range's boundary points, kept in order by §5.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LiveRange {
    pub start: Point,
    pub end: Point,
}

impl LiveRange {
    fn side(&self, start: bool) -> Point {
        if start { self.start } else { self.end }
    }
}

/// Where a range cuts the tree: the common ancestor, the partially contained
/// nodes on each side from just below it down to the boundary's node, and
/// the common ancestor's contained children, by index.
struct Cut {
    ancestor: usize,
    left: Vec<usize>,
    right: Vec<usize>,
    first: usize,
    last: usize,
}

fn invalid_node() -> BindingError {
    dom_error(
        "InvalidNodeTypeError",
        "the node can't hold a range's boundary point",
    )
}

fn index_size() -> BindingError {
    dom_error("IndexSizeError", "the offset is past the end of the node")
}

fn wrong_document() -> BindingError {
    dom_error("WrongDocumentError", "the nodes are in different trees")
}

fn hierarchy(message: &str) -> BindingError {
    dom_error("HierarchyRequestError", message)
}

fn broken() -> BindingError {
    // Only after a walk stopped at the deadline, which ends the script.
    BindingError::TypeError("the range's boundary points are out of date".to_owned())
}

fn utf16_len(s: &str) -> u32 {
    u32::try_from(s.encode_utf16().count()).unwrap_or(u32::MAX)
}

/// `s`'s code units `from..to`, as `Document::substring_data` takes them.
fn utf16_slice(s: &str, from: u32, to: u32) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    let to = (to as usize).min(units.len());
    let from = (from as usize).min(to);
    String::from_utf16_lossy(&units[from..to])
}

impl Document {
    /// The running call's `Range`, as an index into `ranges`.
    fn range_of(&mut self, this: ObjectId) -> BindingResult<usize> {
        match self.object_at(this)? {
            Object::Range(r) => Ok(r as usize),
            _ => Err(BindingError::TypeError("not a Range".to_owned())),
        }
    }

    fn bounds_of(&mut self, this: ObjectId) -> BindingResult<LiveRange> {
        match self.object_at(this)? {
            Object::Range(r) => self.ranges.get(r as usize).copied().ok_or_else(broken),
            Object::StaticRange(b) => Ok(*b),
            _ => Err(BindingError::TypeError("not a range".to_owned())),
        }
    }

    fn bounds(&self, r: usize) -> BindingResult<LiveRange> {
        self.ranges.get(r).copied().ok_or_else(broken)
    }

    /// A new range at `(node, 0)`, or `RangeLimit`.
    pub(crate) fn new_range(&mut self, node: usize) -> Result<Handle, DomError> {
        let limit = self.limits.max_ranges;
        if self.ranges.len() >= limit {
            return Err(DomError::RangeLimit { limit });
        }
        let point = Point { node, offset: 0 };
        self.new_range_at(LiveRange {
            start: point,
            end: point,
        })
    }

    fn new_range_at(&mut self, bounds: LiveRange) -> Result<Handle, DomError> {
        self.handle_for(bounds.start.node)?;
        self.handle_for(bounds.end.node)?;
        let r = u32::try_from(self.ranges.len()).unwrap_or(u32::MAX);
        let h = self.new_handle(Object::Range(r))?;
        self.ranges.push(bounds);
        *self.range_points.entry(bounds.start.node).or_insert(0) += 1;
        *self.range_points.entry(bounds.end.node).or_insert(0) += 1;
        Ok(h)
    }

    /// `new StaticRange`: its boundary points aren't checked against the
    /// tree and never move (DOM §5.4).
    pub(crate) fn static_range(
        &mut self,
        start: ObjectId,
        start_offset: u32,
        end: ObjectId,
        end_offset: u32,
    ) -> BindingResult<ObjectId> {
        let start = self.range_node(start)?;
        let end = self.range_node(end)?;
        if self.kind(start) == Some(Kind::DocumentType)
            || self.kind(end) == Some(Kind::DocumentType)
        {
            return Err(invalid_node());
        }
        let bounds = LiveRange {
            start: Point {
                node: start,
                offset: start_offset,
            },
            end: Point {
                node: end,
                offset: end_offset,
            },
        };
        let pinned = self
            .handle_for(start)
            .and_then(|_| self.handle_for(end))
            .and_then(|_| self.new_handle(Object::StaticRange(Box::new(bounds))));
        match pinned {
            Ok(h) => Ok(ObjectId(u64::from(h))),
            Err(e) => Err(self.fail(e)),
        }
    }

    /// A node argument. An `Attr` isn't a whole `Node` here, and DOM gives
    /// one no place in a tree; a `StaticRange` refuses it, as the standard
    /// says, and a `Range` does too, where the standard would allow offset 0.
    fn range_node(&mut self, id: ObjectId) -> BindingResult<usize> {
        match self.object_at(id)? {
            Object::Node(n) => Ok(n),
            _ => Err(invalid_node()),
        }
    }

    /// Moves one boundary point. `Err` leaves it where it was.
    fn set_point(&mut self, r: usize, start: bool, to: Point) -> Result<(), DomError> {
        self.handle_for(to.node)?;
        let Some(range) = self.ranges.get_mut(r) else {
            return Ok(());
        };
        let slot = if start {
            &mut range.start
        } else {
            &mut range.end
        };
        let from = std::mem::replace(slot, to);
        if from.node != to.node {
            *self.range_points.entry(to.node).or_insert(0) += 1;
            if let Some(n) = self.range_points.get_mut(&from.node) {
                *n -= 1;
                if *n == 0 {
                    self.range_points.remove(&from.node);
                }
            }
        }
        Ok(())
    }

    /// [`set_point`](Self::set_point) from a mutation, which can't refuse:
    /// out of handles, the point stays and the call fails. Its node is alive,
    /// which is all that has to hold.
    fn move_point(&mut self, r: usize, start: bool, to: Point) {
        if let Err(e) = self.set_point(r, start, to) {
            self.fault.get_or_insert(e);
        }
    }

    /// Both boundary points, for the methods that end with a range collapsed
    /// or selecting something.
    fn set_bounds(&mut self, r: usize, start: Point, end: Point) -> BindingResult<()> {
        self.set_point(r, true, start)
            .and_then(|()| self.set_point(r, false, end))
            .map_err(|e| self.fail(e))
    }

    /// DOM §4.2 "length".
    pub(crate) fn node_length(&self, id: usize) -> u32 {
        match self.kind(id) {
            Some(Kind::DocumentType) => 0,
            Some(Kind::Text | Kind::Comment | Kind::ProcessingInstruction) => {
                utf16_len(self.char_data(id))
            }
            _ => u32::try_from(self.children(id).len()).unwrap_or(u32::MAX),
        }
    }

    fn is_character_data(&self, id: usize) -> bool {
        matches!(
            self.kind(id),
            Some(Kind::Text | Kind::Comment | Kind::ProcessingInstruction)
        )
    }

    /// The node's index among its siblings: the sibling cursor spares a loop
    /// along one parent the rescan.
    pub(crate) fn index_of(&self, id: usize) -> usize {
        let Some(parent) = self.parent(id) else {
            return 0;
        };
        let list = self.children(parent);
        let (p, i) = self.cursor.get();
        if p == parent && list.get(i) == Some(&id) {
            return i;
        }
        let at = list.iter().position(|&c| c == id).unwrap_or(0);
        self.cursor.set((parent, at));
        at
    }

    fn root_of(&self, id: usize) -> usize {
        let mut at = id;
        while let Some(p) = self.parent(at) {
            at = p;
        }
        at
    }

    /// §5.2 "position": where `a` is relative to `b`, in one tree.
    fn point_order(&self, a: Point, b: Point) -> Ordering {
        if a.node == b.node {
            return a.offset.cmp(&b.offset);
        }
        let (p, q) = (self.path(a.node), self.path(b.node));
        let shared = p.iter().zip(&q).take_while(|(x, y)| x == y).count();
        match (p.get(shared), q.get(shared)) {
            // `a`'s node contains `b`'s.
            (None, Some(&child)) => {
                if (self.index_of(child) as u64) < u64::from(a.offset) {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (Some(&child), None) => {
                if (self.index_of(child) as u64) < u64::from(b.offset) {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (Some(&x), Some(&y)) => self.index_of(x).cmp(&self.index_of(y)),
            (None, None) => Ordering::Equal,
        }
    }

    /// §5.5 "set the start or end".
    fn set_boundary(
        &mut self,
        r: usize,
        start: bool,
        node: usize,
        offset: u32,
    ) -> BindingResult<()> {
        if self.kind(node) == Some(Kind::DocumentType) {
            return Err(invalid_node());
        }
        if offset > self.node_length(node) {
            return Err(index_size());
        }
        let to = Point { node, offset };
        let range = self.bounds(r)?;
        let other = range.side(!start);
        let wrong_way = |d: &Self| {
            let order = d.point_order(to, other);
            if start {
                order == Ordering::Greater
            } else {
                order == Ordering::Less
            }
        };
        let moved = self.root_of(node) != self.root_of(other.node) || wrong_way(self);
        self.poll_by(self.depth(node))?;
        let r2 = if moved {
            self.set_point(r, !start, to)
        } else {
            Ok(())
        };
        r2.and_then(|()| self.set_point(r, start, to))
            .map_err(|e| self.fail(e))
    }

    /// A node's place in its parent, for the `set*Before` and `select` family.
    fn place_of(&self, node: usize) -> BindingResult<(usize, u32)> {
        let parent = self.parent(node).ok_or_else(invalid_node)?;
        Ok((
            parent,
            u32::try_from(self.index_of(node)).unwrap_or(u32::MAX),
        ))
    }

    /// See [`Cut`]. `None` if the points aren't in one tree, which only a walk
    /// stopped at the deadline can leave.
    fn cut(&self, range: LiveRange) -> Option<Cut> {
        let (p, q) = (self.path(range.start.node), self.path(range.end.node));
        let shared = p.iter().zip(&q).take_while(|(x, y)| x == y).count();
        let ancestor = *p.get(shared.checked_sub(1)?)?;
        let left = p[shared..].to_vec();
        let right = q[shared..].to_vec();
        let first = left
            .first()
            .map_or(range.start.offset as usize, |&n| self.index_of(n) + 1);
        let last = right
            .first()
            .map_or(range.end.offset as usize, |&n| self.index_of(n));
        Some(Cut {
            ancestor,
            left,
            right,
            first,
            last,
        })
    }

    /// The children of `id` at `from..to`, clamped.
    fn child_span(&self, id: usize, from: usize, to: usize) -> Vec<usize> {
        let list = self.children(id);
        let to = to.min(list.len());
        list.get(from.min(to)..to).unwrap_or_default().to_vec()
    }

    /// The children of `id` after `child`.
    fn after_child(&self, id: usize, child: usize) -> Vec<usize> {
        self.child_span(id, self.index_of(child) + 1, usize::MAX)
    }

    /// The children of `id` before `child`.
    fn before_child(&self, id: usize, child: usize) -> Vec<usize> {
        self.child_span(id, 0, self.index_of(child))
    }

    /// `node`'s data from `from` to `to`, in a copy of `node`.
    fn data_copy(&mut self, node: usize, from: u32, to: u32) -> BindingResult<usize> {
        let copy = self.clone_tree(node, false)?;
        let data = utf16_slice(self.char_data(node), from, to);
        self.write_char_data(copy, &data).map_err(|e| self.fail(e))?;
        Ok(copy)
    }

    /// §5.5 "clone the contents". Partially contained nodes are walked as two
    /// chains, not by recursion: a chain is as long as the tree is deep.
    fn clone_range_contents(&mut self, bounds: LiveRange) -> BindingResult<usize> {
        let frag = self.create_fragment().map_err(|e| self.fail(e))?;
        let (s, e) = (bounds.start, bounds.end);
        if s == e {
            return Ok(frag);
        }
        if s.node == e.node && self.is_character_data(s.node) {
            let copy = self.data_copy(s.node, s.offset, e.offset)?;
            self.place(frag, &[copy], None);
            return Ok(frag);
        }
        let cut = self.cut(bounds).ok_or_else(broken)?;
        let contained = self.child_span(cut.ancestor, cut.first, cut.last);
        self.refuse_doctype(&contained)?;
        let mut pending = Vec::with_capacity(cut.left.len());
        let mut parent = frag;
        for (k, &node) in cut.left.iter().enumerate() {
            let deepest = k + 1 == cut.left.len();
            let copy = if deepest && self.is_character_data(node) {
                self.data_copy(node, s.offset, u32::MAX)?
            } else {
                self.clone_tree(node, false)?
            };
            self.place(parent, &[copy], None);
            // What follows the chain: its next level's copy is already first
            // in `copy` when this goes in behind it.
            let rest = match cut.left.get(k + 1) {
                Some(&next) => {
                    parent = copy;
                    self.after_child(node, next)
                }
                None if self.is_character_data(node) => Vec::new(),
                None => self.child_span(node, s.offset as usize, usize::MAX),
            };
            pending.push((copy, rest));
        }
        for (copy, rest) in pending.into_iter().rev() {
            self.clone_into(copy, &rest)?;
        }
        self.clone_into(frag, &contained)?;
        let mut parent = frag;
        for (k, &node) in cut.right.iter().enumerate() {
            let deepest = k + 1 == cut.right.len();
            let copy = if deepest && self.is_character_data(node) {
                self.data_copy(node, 0, e.offset)?
            } else {
                self.clone_tree(node, false)?
            };
            self.place(parent, &[copy], None);
            let rest = match cut.right.get(k + 1) {
                Some(&next) => self.before_child(node, next),
                None if self.is_character_data(node) => Vec::new(),
                None => self.child_span(node, 0, e.offset as usize),
            };
            self.clone_into(copy, &rest)?;
            parent = copy;
        }
        Ok(frag)
    }

    fn clone_into(&mut self, parent: usize, nodes: &[usize]) -> BindingResult<()> {
        for &n in nodes {
            self.poll()?;
            let copy = self.clone_tree(n, true)?;
            self.place(parent, &[copy], None);
        }
        Ok(())
    }

    fn refuse_doctype(&self, nodes: &[usize]) -> BindingResult<()> {
        if nodes
            .iter()
            .any(|&n| self.kind(n) == Some(Kind::DocumentType))
        {
            return Err(hierarchy("a range can't take a doctype"));
        }
        Ok(())
    }

    /// Appends `nodes` to `parent`, each leaving where it was.
    fn move_into(&mut self, parent: usize, nodes: &[usize]) -> BindingResult<()> {
        for &n in nodes {
            if let Some(from) = self.parent(n) {
                self.poll_by(self.children(from).len())?;
            }
            self.pre_insert(parent, n, None)?;
        }
        Ok(())
    }

    /// Where a range ends up after its contents leave: the steps "new node"
    /// and "new offset" of §5.5 "extract" and `deleteContents`.
    fn after_removal(&self, s: Point, e: Point) -> Point {
        if self.is_inclusive_ancestor(s.node, e.node) {
            return s;
        }
        let mut reference = s.node;
        while let Some(p) = self.parent(reference) {
            if self.is_inclusive_ancestor(p, e.node) {
                let index = u32::try_from(self.index_of(reference)).unwrap_or(u32::MAX);
                return Point {
                    node: p,
                    offset: index.saturating_add(1),
                };
            }
            reference = p;
        }
        s
    }

    /// §5.5 "extract".
    fn extract_range(&mut self, r: usize) -> BindingResult<usize> {
        let bounds = self.bounds(r)?;
        let (s, e) = (bounds.start, bounds.end);
        let frag = self.create_fragment().map_err(|e| self.fail(e))?;
        if s == e {
            return Ok(frag);
        }
        if s.node == e.node && self.is_character_data(s.node) {
            let copy = self.data_copy(s.node, s.offset, e.offset)?;
            self.place(frag, &[copy], None);
            self.replace_data(s.node, s.offset, e.offset - s.offset, "")?;
            return Ok(frag);
        }
        let cut = self.cut(bounds).ok_or_else(broken)?;
        let contained = self.child_span(cut.ancestor, cut.first, cut.last);
        self.refuse_doctype(&contained)?;
        let new_point = self.after_removal(s, e);

        // The left chain: copies go in from the top, and contents leave from
        // the bottom, the order §5.5's recursion has.
        let mut parent = frag;
        let mut copies = Vec::with_capacity(cut.left.len());
        for &node in &cut.left {
            let copy = if node == s.node && self.is_character_data(node) {
                self.data_copy(node, s.offset, u32::MAX)?
            } else {
                self.clone_tree(node, false)?
            };
            self.place(parent, &[copy], None);
            copies.push(copy);
            parent = copy;
        }
        if let (Some(&deepest), Some(&copy)) = (cut.left.last(), copies.last()) {
            if self.is_character_data(deepest) {
                let length = self.node_length(deepest);
                self.replace_data(deepest, s.offset, length.saturating_sub(s.offset), "")?;
            } else {
                let rest = self.child_span(deepest, s.offset as usize, usize::MAX);
                self.move_into(copy, &rest)?;
            }
        }
        for k in (0..cut.left.len().saturating_sub(1)).rev() {
            let rest = self.after_child(cut.left[k], cut.left[k + 1]);
            self.move_into(copies[k], &rest)?;
        }
        self.move_into(frag, &contained)?;

        // The right chain goes from the top: what comes before the chain's
        // next node leaves first.
        let mut parent = frag;
        for (k, &node) in cut.right.iter().enumerate() {
            let last_one = k + 1 == cut.right.len();
            let copy = if last_one && self.is_character_data(node) {
                self.data_copy(node, 0, e.offset)?
            } else {
                self.clone_tree(node, false)?
            };
            self.place(parent, &[copy], None);
            parent = copy;
            match cut.right.get(k + 1) {
                Some(&next) => {
                    let rest = self.before_child(node, next);
                    self.move_into(copy, &rest)?;
                }
                None if self.is_character_data(node) => {
                    self.replace_data(node, 0, e.offset, "")?;
                }
                None => {
                    let rest = self.child_span(node, 0, e.offset as usize);
                    self.move_into(copy, &rest)?;
                }
            }
        }
        self.set_bounds(r, new_point, new_point)?;
        Ok(frag)
    }

    /// `deleteContents`, §5.5.
    fn delete_range(&mut self, r: usize) -> BindingResult<()> {
        let bounds = self.bounds(r)?;
        let (s, e) = (bounds.start, bounds.end);
        if s == e {
            return Ok(());
        }
        if s.node == e.node && self.is_character_data(s.node) {
            return self.replace_data(s.node, s.offset, e.offset - s.offset, "");
        }
        let cut = self.cut(bounds).ok_or_else(broken)?;
        // Contained nodes, in tree order, without those whose parent is too.
        let mut gone = Vec::new();
        if let Some(&deepest) = cut.left.last()
            && !self.is_character_data(deepest)
        {
            gone.extend(self.child_span(deepest, s.offset as usize, usize::MAX));
        }
        for pair in cut.left.windows(2).rev() {
            gone.extend(self.after_child(pair[0], pair[1]));
        }
        gone.extend(self.child_span(cut.ancestor, cut.first, cut.last));
        for pair in cut.right.windows(2) {
            gone.extend(self.before_child(pair[0], pair[1]));
        }
        if let Some(&deepest) = cut.right.last()
            && !self.is_character_data(deepest)
        {
            gone.extend(self.child_span(deepest, 0, e.offset as usize));
        }
        let new_point = self.after_removal(s, e);
        if self.is_character_data(s.node) {
            let length = self.node_length(s.node);
            self.replace_data(s.node, s.offset, length.saturating_sub(s.offset), "")?;
        }
        for n in gone {
            if let Some(parent) = self.parent(n) {
                self.poll_by(self.children(parent).len())?;
                self.pre_remove(parent, n)?;
            }
        }
        if self.is_character_data(e.node) {
            self.replace_data(e.node, 0, e.offset, "")?;
        }
        self.set_bounds(r, new_point, new_point)
    }

    /// §5.5 "insert".
    fn insert_in_range(&mut self, r: usize, node: usize) -> BindingResult<()> {
        let start = self.bounds(r)?.start;
        let start_kind = self.kind(start.node);
        let lone_text = start_kind == Some(Kind::Text) && self.parent(start.node).is_none();
        if matches!(
            start_kind,
            Some(Kind::Comment | Kind::ProcessingInstruction)
        ) || lone_text
            || start.node == node
        {
            return Err(hierarchy("a range can't insert at its start"));
        }
        let mut reference = if start_kind == Some(Kind::Text) {
            Some(start.node)
        } else {
            self.children(start.node).get(start.offset as usize).copied()
        };
        let parent = match reference {
            Some(c) => self.parent(c).ok_or_else(broken)?,
            None => start.node,
        };
        self.check_insert(parent, node, reference, None)?;
        if start_kind == Some(Kind::Text) {
            reference = Some(self.split_text(start.node, start.offset)?);
        }
        if reference == Some(node) {
            reference = self.sibling(node, true);
        }
        if let Some(from) = self.parent(node) {
            self.pre_remove(from, node)?;
        }
        let mut offset = match reference {
            Some(c) => u32::try_from(self.index_of(c)).unwrap_or(u32::MAX),
            None => self.node_length(parent),
        };
        offset = offset.saturating_add(if self.kind(node) == Some(Kind::DocumentFragment) {
            self.node_length(node)
        } else {
            1
        });
        self.pre_insert(parent, node, reference)?;
        let now = self.bounds(r)?;
        if now.start == now.end {
            let end = Point {
                node: parent,
                offset,
            };
            self.set_point(r, false, end).map_err(|e| self.fail(e))?;
        }
        Ok(())
    }

    /// The text of the `Text` nodes under `top`, itself included.
    fn push_text_under(&self, top: usize, out: &mut String) -> BindingResult<()> {
        let mut stack = vec![top];
        while let Some(n) = stack.pop() {
            self.poll()?;
            if self.kind(n) == Some(Kind::Text) {
                out.push_str(self.char_data(n));
            } else {
                stack.extend(self.children(n).iter().rev().copied());
            }
        }
        Ok(())
    }

    /// §5.5's stringifier.
    fn range_text(&self, bounds: LiveRange) -> BindingResult<String> {
        let (s, e) = (bounds.start, bounds.end);
        let is_text = |d: &Self, n| d.kind(n) == Some(Kind::Text);
        if s.node == e.node && is_text(self, s.node) {
            return Ok(utf16_slice(self.char_data(s.node), s.offset, e.offset));
        }
        let cut = self.cut(bounds).ok_or_else(broken)?;
        let mut out = String::new();
        if is_text(self, s.node) {
            out = utf16_slice(self.char_data(s.node), s.offset, u32::MAX);
        }
        let mut order = Vec::new();
        if let Some(&deepest) = cut.left.last()
            && !self.is_character_data(deepest)
        {
            order.extend(self.child_span(deepest, s.offset as usize, usize::MAX));
        }
        for pair in cut.left.windows(2).rev() {
            order.extend(self.after_child(pair[0], pair[1]));
        }
        order.extend(self.child_span(cut.ancestor, cut.first, cut.last));
        for pair in cut.right.windows(2) {
            order.extend(self.before_child(pair[0], pair[1]));
        }
        if let Some(&deepest) = cut.right.last()
            && !self.is_character_data(deepest)
        {
            order.extend(self.child_span(deepest, 0, e.offset as usize));
        }
        for n in order {
            self.push_text_under(n, &mut out)?;
        }
        if is_text(self, e.node) {
            out.push_str(&utf16_slice(self.char_data(e.node), 0, e.offset));
        }
        Ok(out)
    }

    /// A range method's point: what `isPointInRange` and `comparePoint` check.
    fn checked_point(&self, node: usize, offset: u32) -> BindingResult<Point> {
        if self.kind(node) == Some(Kind::DocumentType) {
            return Err(invalid_node());
        }
        if offset > self.node_length(node) {
            return Err(index_size());
        }
        Ok(Point { node, offset })
    }

    // What mutations do to live ranges (§4.2.3 "insert" and "remove", §4.10
    // "replace data", §4.11 "split", §4.4 `normalize`).

    fn range_touches(&self, node: usize) -> bool {
        self.range_points.contains_key(&node)
    }

    /// Whether a range has a boundary point on `id`, for a mutation to skip
    /// working out the lengths.
    pub(crate) fn has_range_point(&self, id: usize) -> bool {
        !self.ranges.is_empty() && self.range_touches(id)
    }

    /// Every boundary point on `node`, with its range's number and side.
    fn points_on(&self, node: usize) -> Vec<(usize, bool, u32)> {
        let mut out = Vec::new();
        for (r, range) in self.ranges.iter().enumerate() {
            if range.start.node == node {
                out.push((r, true, range.start.offset));
            }
            if range.end.node == node {
                out.push((r, false, range.end.offset));
            }
        }
        out
    }

    /// `count` nodes join `parent` before `before`, which is still there.
    pub(crate) fn ranges_inserting(
        &mut self,
        parent: usize,
        before: Option<usize>,
        count: usize,
    ) {
        // With no `before` they go last, past every offset.
        let Some(before) = before else { return };
        if !self.has_range_point(parent) {
            return;
        }
        let index = self.index_of(before) as u64;
        let count = u32::try_from(count).unwrap_or(u32::MAX);
        for (r, start, offset) in self.points_on(parent) {
            if u64::from(offset) > index {
                let offset = offset.saturating_add(count);
                self.move_point(r, start, Point { node: parent, offset });
            }
        }
    }

    /// Whether a boundary point may be in the subtrees under `gone`: a walk
    /// no longer than the range list, so a removal of a small subtree doesn't
    /// walk every range's ancestors.
    fn point_may_be_under(&self, gone: &[usize]) -> bool {
        let mut budget = self.ranges.len().saturating_mul(2).saturating_add(gone.len());
        let mut stack = gone.to_vec();
        while let Some(n) = stack.pop() {
            if self.range_touches(n) {
                return true;
            }
            budget = match budget.checked_sub(1) {
                Some(b) => b,
                None => return true,
            };
            stack.extend_from_slice(self.children(n));
        }
        false
    }

    /// `gone`, children of `parent`, are about to leave it. The steps of
    /// §4.2.3 "remove" for all of them at once: one pass over the ranges
    /// however many leave.
    pub(crate) fn ranges_removing(&mut self, parent: usize, gone: &[usize]) {
        if self.ranges.is_empty()
            || gone.is_empty()
            || !(self.range_touches(parent) || self.point_may_be_under(gone))
        {
            return;
        }
        let children = self.children(parent);
        // How many of the first `i` children leave, and each child's index.
        let leaving: HashMap<usize, ()> = gone.iter().map(|&n| (n, ())).collect();
        let mut gone_before = Vec::with_capacity(children.len() + 1);
        let mut count = 0_u32;
        let mut index = HashMap::with_capacity(children.len());
        for (i, &c) in children.iter().enumerate() {
            gone_before.push(count);
            index.insert(c, i);
            count += u32::from(leaving.contains_key(&c));
        }
        gone_before.push(count);
        for r in 0..self.ranges.len() {
            for start in [true, false] {
                let Ok(range) = self.bounds(r) else {
                    return;
                };
                let p = range.side(start);
                if p.node == parent {
                    let at = (p.offset as usize).min(children_len(&gone_before));
                    let offset = p.offset - gone_before[at];
                    if offset != p.offset {
                        self.move_point(r, start, Point { node: parent, offset });
                    }
                    continue;
                }
                // An inclusive descendant of a child that leaves?
                let mut at = p.node;
                while let Some(up) = self.parent(at) {
                    if self.poll().is_err() {
                        return;
                    }
                    if up == parent {
                        if leaving.contains_key(&at)
                            && let Some(&i) = index.get(&at)
                        {
                            let offset = i as u32 - gone_before[i];
                            self.move_point(r, start, Point { node: parent, offset });
                        }
                        break;
                    }
                    at = up;
                }
            }
        }
    }

    /// §4.10 "replace data": `count` units at `offset` became `length` units.
    pub(crate) fn ranges_replaced_data(
        &mut self,
        node: usize,
        offset: u32,
        count: u32,
        length: u32,
    ) {
        if !self.has_range_point(node) {
            return;
        }
        let end = u64::from(offset) + u64::from(count);
        for (r, start, at) in self.points_on(node) {
            let at64 = u64::from(at);
            let moved = if at > offset && at64 <= end {
                offset
            } else if at64 > end {
                (at64 + u64::from(length) - u64::from(count)) as u32
            } else {
                continue;
            };
            self.move_point(
                r,
                start,
                Point {
                    node,
                    offset: moved,
                },
            );
        }
    }

    /// §4.11 "split": `new`, the tail of `node` from `at`, has just gone in
    /// after `node`, under `parent`.
    pub(crate) fn ranges_split(&mut self, node: usize, new: usize, parent: usize, at: u32) {
        if self.has_range_point(node) {
            for (r, start, offset) in self.points_on(node) {
                if offset > at {
                    let to = Point {
                        node: new,
                        offset: offset - at,
                    };
                    self.move_point(r, start, to);
                }
            }
        }
        if self.has_range_point(parent) {
            let index = self.index_of(node) as u64;
            for (r, start, offset) in self.points_on(parent) {
                if u64::from(offset) == index + 1 {
                    let to = Point {
                        node: parent,
                        offset: offset + 1,
                    };
                    self.move_point(r, start, to);
                }
            }
        }
    }

    /// §4.4 `normalize`, step 6.5: `merged` is about to join `node`, which
    /// has `length` units before it.
    pub(crate) fn ranges_merging(&mut self, node: usize, merged: usize, length: u32) {
        if self.has_range_point(merged) {
            for (r, start, offset) in self.points_on(merged) {
                let to = Point {
                    node,
                    offset: offset.saturating_add(length),
                };
                self.move_point(r, start, to);
            }
        }
        let Some(parent) = self.parent(merged) else {
            return;
        };
        if self.has_range_point(parent) {
            let index = self.index_of(merged) as u64;
            for (r, start, offset) in self.points_on(parent) {
                if u64::from(offset) == index {
                    self.move_point(
                        r,
                        start,
                        Point {
                            node,
                            offset: length,
                        },
                    );
                }
            }
        }
    }
}

fn children_len(gone_before: &[u32]) -> usize {
    gone_before.len().saturating_sub(1)
}

impl idl::AbstractRange for Document {
    fn start_container(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let node = self.bounds_of(this)?.start.node;
        self.object(node)
    }

    fn start_offset(&mut self, this: ObjectId) -> BindingResult<u32> {
        Ok(self.bounds_of(this)?.start.offset)
    }

    fn end_container(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let node = self.bounds_of(this)?.end.node;
        self.object(node)
    }

    fn end_offset(&mut self, this: ObjectId) -> BindingResult<u32> {
        Ok(self.bounds_of(this)?.end.offset)
    }

    fn collapsed(&mut self, this: ObjectId) -> BindingResult<bool> {
        let b = self.bounds_of(this)?;
        Ok(b.start == b.end)
    }
}

impl idl::Range for Document {
    fn constructor(&mut self) -> BindingResult<ObjectId> {
        let root = self.root();
        match self.new_range(root) {
            Ok(h) => Ok(ObjectId(u64::from(h))),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn common_ancestor_container(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let bounds = self.bounds_of(this)?;
        let cut = self.cut(bounds).ok_or_else(broken)?;
        self.object(cut.ancestor)
    }

    fn set_start(&mut self, this: ObjectId, node: ObjectId, offset: u32) -> BindingResult<()> {
        let r = self.range_of(this)?;
        let node = self.range_node(node)?;
        self.set_boundary(r, true, node, offset)
    }

    fn set_end(&mut self, this: ObjectId, node: ObjectId, offset: u32) -> BindingResult<()> {
        let r = self.range_of(this)?;
        let node = self.range_node(node)?;
        self.set_boundary(r, false, node, offset)
    }

    fn set_start_before(&mut self, this: ObjectId, node: ObjectId) -> BindingResult<()> {
        let r = self.range_of(this)?;
        let node = self.range_node(node)?;
        let (parent, index) = self.place_of(node)?;
        self.set_boundary(r, true, parent, index)
    }

    fn set_start_after(&mut self, this: ObjectId, node: ObjectId) -> BindingResult<()> {
        let r = self.range_of(this)?;
        let node = self.range_node(node)?;
        let (parent, index) = self.place_of(node)?;
        self.set_boundary(r, true, parent, index + 1)
    }

    fn set_end_before(&mut self, this: ObjectId, node: ObjectId) -> BindingResult<()> {
        let r = self.range_of(this)?;
        let node = self.range_node(node)?;
        let (parent, index) = self.place_of(node)?;
        self.set_boundary(r, false, parent, index)
    }

    fn set_end_after(&mut self, this: ObjectId, node: ObjectId) -> BindingResult<()> {
        let r = self.range_of(this)?;
        let node = self.range_node(node)?;
        let (parent, index) = self.place_of(node)?;
        self.set_boundary(r, false, parent, index + 1)
    }

    fn collapse(&mut self, this: ObjectId, to_start: bool) -> BindingResult<()> {
        let r = self.range_of(this)?;
        let bounds = self.bounds(r)?;
        let at = bounds.side(to_start);
        self.set_bounds(r, at, at)
    }

    fn select_node(&mut self, this: ObjectId, node: ObjectId) -> BindingResult<()> {
        let r = self.range_of(this)?;
        let node = self.range_node(node)?;
        let (parent, index) = self.place_of(node)?;
        let start = Point {
            node: parent,
            offset: index,
        };
        let end = Point {
            node: parent,
            offset: index + 1,
        };
        self.set_bounds(r, start, end)
    }

    fn select_node_contents(&mut self, this: ObjectId, node: ObjectId) -> BindingResult<()> {
        let r = self.range_of(this)?;
        let node = self.range_node(node)?;
        if self.kind(node) == Some(Kind::DocumentType) {
            return Err(invalid_node());
        }
        let start = Point { node, offset: 0 };
        let end = Point {
            node,
            offset: self.node_length(node),
        };
        self.set_bounds(r, start, end)
    }

    fn compare_boundary_points(
        &mut self,
        this: ObjectId,
        how: u16,
        source_range: ObjectId,
    ) -> BindingResult<i16> {
        if how > 3 {
            return Err(dom_error(
                "NotSupportedError",
                "how must be one of Range's four comparison constants",
            ));
        }
        let a = self.bounds_of(this)?;
        let b = self.bounds_of(source_range)?;
        if self.root_of(a.start.node) != self.root_of(b.start.node) {
            return Err(wrong_document());
        }
        // START_TO_START, START_TO_END, END_TO_END, END_TO_START.
        let (x, y) = match how {
            0 => (a.start, b.start),
            1 => (a.end, b.start),
            2 => (a.end, b.end),
            _ => (a.start, b.end),
        };
        Ok(self.point_order(x, y) as i16)
    }

    fn delete_contents(&mut self, this: ObjectId) -> BindingResult<()> {
        let r = self.range_of(this)?;
        self.delete_range(r)
    }

    fn extract_contents(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let r = self.range_of(this)?;
        let owner = self.owner_of_node(self.bounds(r)?.start.node);
        let frag = self.making_in(owner, |d| d.extract_range(r))?;
        self.object(frag)
    }

    fn clone_contents(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let bounds = self.bounds_of(this)?;
        let owner = self.owner_of_node(bounds.start.node);
        let frag = self.making_in(owner, |d| d.clone_range_contents(bounds))?;
        self.object(frag)
    }

    fn insert_node(&mut self, this: ObjectId, node: ObjectId) -> BindingResult<()> {
        let r = self.range_of(this)?;
        let node = self.range_node(node)?;
        self.insert_in_range(r, node)
    }

    fn surround_contents(&mut self, this: ObjectId, new_parent: ObjectId) -> BindingResult<()> {
        let r = self.range_of(this)?;
        let new_parent = self.range_node(new_parent)?;
        let bounds = self.bounds(r)?;
        let cut = self.cut(bounds).ok_or_else(broken)?;
        let partial = cut.left.iter().chain(&cut.right);
        if partial.into_iter().any(|&n| self.kind(n) != Some(Kind::Text)) {
            return Err(dom_error(
                "InvalidStateError",
                "the range partially selects a node that isn't text",
            ));
        }
        if matches!(
            self.kind(new_parent),
            Some(Kind::Document | Kind::DocumentType | Kind::DocumentFragment)
        ) {
            return Err(invalid_node());
        }
        let owner = self.owner_of_node(bounds.start.node);
        let frag = self.making_in(owner, |d| d.extract_range(r))?;
        if !self.children(new_parent).is_empty() {
            self.replace_all(new_parent, None);
        }
        self.insert_in_range(r, new_parent)?;
        self.pre_insert(new_parent, frag, None)?;
        self.drop_subtree(frag);
        let (parent, index) = self.place_of(new_parent)?;
        let start = Point {
            node: parent,
            offset: index,
        };
        let end = Point {
            node: parent,
            offset: index + 1,
        };
        self.set_bounds(r, start, end)
    }

    fn clone_range(&mut self, this: ObjectId) -> BindingResult<ObjectId> {
        let bounds = self.bounds_of(this)?;
        let limit = self.limits.max_ranges;
        if self.ranges.len() >= limit {
            return Err(self.fail(DomError::RangeLimit { limit }));
        }
        match self.new_range_at(bounds) {
            Ok(h) => Ok(ObjectId(u64::from(h))),
            Err(e) => Err(self.fail(e)),
        }
    }

    fn detach(&mut self, this: ObjectId) -> BindingResult<()> {
        // §5.5: does nothing, for compatibility.
        self.range_of(this).map(|_| ())
    }

    fn is_point_in_range(
        &mut self,
        this: ObjectId,
        node: ObjectId,
        offset: u32,
    ) -> BindingResult<bool> {
        let bounds = self.bounds_of(this)?;
        let node = self.range_node(node)?;
        if self.root_of(node) != self.root_of(bounds.start.node) {
            return Ok(false);
        }
        let p = self.checked_point(node, offset)?;
        Ok(self.point_order(p, bounds.start) != Ordering::Less
            && self.point_order(p, bounds.end) != Ordering::Greater)
    }

    fn compare_point(&mut self, this: ObjectId, node: ObjectId, offset: u32) -> BindingResult<i16> {
        let bounds = self.bounds_of(this)?;
        let node = self.range_node(node)?;
        if self.root_of(node) != self.root_of(bounds.start.node) {
            return Err(wrong_document());
        }
        let p = self.checked_point(node, offset)?;
        if self.point_order(p, bounds.start) == Ordering::Less {
            return Ok(-1);
        }
        Ok(i16::from(
            self.point_order(p, bounds.end) == Ordering::Greater,
        ))
    }

    fn intersects_node(&mut self, this: ObjectId, node: ObjectId) -> BindingResult<bool> {
        let bounds = self.bounds_of(this)?;
        let node = self.range_node(node)?;
        if self.root_of(node) != self.root_of(bounds.start.node) {
            return Ok(false);
        }
        let Some(parent) = self.parent(node) else {
            return Ok(true);
        };
        let offset = u32::try_from(self.index_of(node)).unwrap_or(u32::MAX);
        let before = Point {
            node: parent,
            offset,
        };
        let after = Point {
            node: parent,
            offset: offset.saturating_add(1),
        };
        Ok(self.point_order(before, bounds.end) == Ordering::Less
            && self.point_order(after, bounds.start) == Ordering::Greater)
    }

    fn create_contextual_fragment(
        &mut self,
        this: ObjectId,
        string: Value,
    ) -> BindingResult<ObjectId> {
        let node = self.bounds_of(this)?.start.node;
        // HTML §8.5 `createContextualFragment`.
        let element = match self.kind(node) {
            Some(Kind::Element) => Some(node),
            Some(Kind::Text | Kind::Comment) => {
                self.parent(node).filter(|&p| self.kind(p) == Some(Kind::Element))
            }
            _ => None,
        };
        let is_html_root = |d: &Self, e: usize| {
            d.in_html_document(e)
                && d.element_name(e)
                    .is_some_and(|n| n.ns == ns!(html) && n.local.as_ref() == "html")
        };
        let context = element.filter(|&e| !is_html_root(self, e));
        let text = match string {
            Value::String(s) => s,
            _ => String::new(),
        };
        let owner = self.owner_of_node(node);
        let parsed = self.making_in(owner, |d| d.parse_fragment_in(context, node, &text))?;
        let Some(frag) = parsed else {
            // The time limit passed in the parse: the script stops here.
            return Err(crate::tree::timed_out());
        };
        self.object(frag)
    }

    fn stringifier(&mut self, this: ObjectId) -> BindingResult<String> {
        let bounds = self.bounds_of(this)?;
        self.range_text(bounds)
    }
}
