//! DOM §6.1 `NodeIterator`: what Rust must keep because Rust sees the removals.
//! The traversal is in `glue_traversal.js`, since a filter is a script's function.

use std::collections::HashMap;

use surimono_webidl::runtime::{BindingError, BindingResult, ObjectId};

use crate::document::Document;

/// What one iterator is charged against `max_text_bytes`. Iterators live until the
/// document drops, like ranges, and every removal looks at each.
const ITERATOR_COST: usize = 4096;

/// A `NodeIterator`'s pointer. Its nodes have handles, so their slots aren't reused.
pub(crate) struct LiveIterator {
    root: usize,
    reference: usize,
    before: bool,
    /// The node a filter is looking at, which the steps follow too: a filter that
    /// removes it leaves the walk on what took its place. Chrome's "candidate node".
    trial: Option<(usize, bool)>,
}

/// Each child leaving at once with its nearest sibling on either side that stays,
/// made once per removal: a list of `n` children leaving under `m` iterators stays
/// `n + m`. Only looked up, so hash order can't reach any output.
type Neighbours = HashMap<usize, (Option<usize>, Option<usize>)>;

fn bad_iterator() -> BindingError {
    BindingError::TypeError("not an iterator".to_owned())
}

impl Document {
    /// A new iterator at `this`; its number.
    pub(crate) fn iterator_new(&mut self, this: ObjectId) -> BindingResult<u32> {
        let root = self.node_of(this)?;
        if self.charge_record(ITERATOR_COST).is_none() {
            // The fault is set, and it is what the call ends with.
            return Err(bad_iterator());
        }
        self.iterators.push(LiveIterator {
            root,
            reference: root,
            before: true,
            trial: None,
        });
        Ok(self.iterators.len() as u32 - 1)
    }

    pub(crate) fn iterator_reference(&mut self, at: u32) -> BindingResult<ObjectId> {
        let id = self
            .iterators
            .get(at as usize)
            .ok_or_else(bad_iterator)?
            .reference;
        self.object(id)
    }

    pub(crate) fn iterator_before(&mut self, at: u32) -> BindingResult<bool> {
        let it = self.iterators.get(at as usize).ok_or_else(bad_iterator)?;
        Ok(it.before)
    }

    pub(crate) fn iterator_set(
        &mut self,
        at: u32,
        node: ObjectId,
        before: bool,
    ) -> BindingResult<()> {
        let id = self.node_of(node)?;
        let it = self
            .iterators
            .get_mut(at as usize)
            .ok_or_else(bad_iterator)?;
        it.reference = id;
        it.before = before;
        Ok(())
    }

    /// The pointer a filter is about to be asked about.
    pub(crate) fn iterator_try(
        &mut self,
        at: u32,
        node: ObjectId,
        before: bool,
    ) -> BindingResult<()> {
        let id = self.node_of(node)?;
        let it = self
            .iterators
            .get_mut(at as usize)
            .ok_or_else(bad_iterator)?;
        it.trial = Some((id, before));
        Ok(())
    }

    pub(crate) fn iterator_trial_node(&mut self, at: u32) -> BindingResult<ObjectId> {
        let it = self.iterators.get(at as usize).ok_or_else(bad_iterator)?;
        let (id, _) = it.trial.ok_or_else(bad_iterator)?;
        self.object(id)
    }

    pub(crate) fn iterator_trial_before(&mut self, at: u32) -> BindingResult<bool> {
        let it = self.iterators.get(at as usize).ok_or_else(bad_iterator)?;
        Ok(it.trial.is_some_and(|(_, before)| before))
    }

    /// `gone`, children of `parent`, are about to leave it, in that order: §6.1
    /// "pre-removing steps" for each iterator, with the result of removing them
    /// one at a time. An iterator is affected by the child of `parent` over its
    /// reference, which a walk up finds.
    ///
    /// A script chooses the length of `gone`, the width of the tree and the number
    /// of iterators, so every walk here is under `poll`. A walk the time limit stops
    /// leaves a pointer out of date in a script that is ending, as for ranges.
    pub(crate) fn iterators_removing(&mut self, parent: usize, gone: &[usize]) {
        if self.iterators.is_empty() || gone.is_empty() {
            return;
        }
        let mut neighbours = None;
        for i in 0..self.iterators.len() {
            let it = &self.iterators[i];
            let (root, pointer, trial) = (it.root, (it.reference, it.before), it.trial);
            let Some(moved) = self.pointer_moved(root, parent, gone, &mut neighbours, pointer)
            else {
                return;
            };
            let moved_trial = match trial {
                Some(t) => match self.pointer_moved(root, parent, gone, &mut neighbours, t) {
                    Some(m) => Some(m),
                    None => return,
                },
                None => None,
            };
            let it = &mut self.iterators[i];
            (it.reference, it.before) = moved;
            it.trial = moved_trial;
        }
    }

    /// [`Neighbours`] of `gone`. `None` if the time limit stopped the walk.
    fn neighbours(&self, parent: usize, gone: &[usize]) -> Option<Neighbours> {
        let mut out: Neighbours = HashMap::with_capacity(gone.len());
        for &n in gone {
            self.poll().ok()?;
            out.insert(n, (None, None));
        }
        let siblings = self.children(parent);
        let mut last = None;
        for &c in siblings {
            self.poll().ok()?;
            match out.get_mut(&c) {
                Some(entry) => entry.0 = last,
                None => last = Some(c),
            }
        }
        last = None;
        for &c in siblings.iter().rev() {
            self.poll().ok()?;
            match out.get_mut(&c) {
                Some(entry) => entry.1 = last,
                None => last = Some(c),
            }
        }
        Some(out)
    }

    /// Where pointer `(node, before)` is once `gone` has left `parent`, with a
    /// handle; `None` if a limit stopped the walk.
    fn pointer_moved(
        &mut self,
        root: usize,
        parent: usize,
        gone: &[usize],
        neighbours: &mut Option<Neighbours>,
        (reference, before): (usize, bool),
    ) -> Option<(usize, bool)> {
        let mut top = reference;
        let found = loop {
            self.poll().ok()?;
            match self.parent(top) {
                Some(p) if p == parent => break true,
                Some(p) => top = p,
                None => break false,
            }
        };
        // One child leaving, the usual case, costs no list of them.
        if !found || (gone.len() == 1 && gone[0] != top) {
            return Some((reference, before));
        }
        if neighbours.is_none() {
            *neighbours = Some(self.neighbours(parent, gone)?);
        }
        let around = neighbours.as_ref()?.get(&top).copied();
        let Some((previous, next)) = around else {
            return Some((reference, before));
        };
        if self.is_inclusive_ancestor(top, root) {
            return Some((reference, before));
        }
        let moved = self.pointer_after_removal(root, parent, previous, next, before)?;
        if let Err(e) = self.handle_for(moved.0) {
            self.fault.get_or_insert(e);
            return None;
        }
        Some(moved)
    }

    /// Where a pointer goes when the child of `parent` it is in leaves, `previous`
    /// and `next` being that child's neighbours that stay; `None` if the time limit
    /// stopped the walk.
    fn pointer_after_removal(
        &self,
        root: usize,
        parent: usize,
        previous: Option<usize>,
        next: Option<usize>,
        before: bool,
    ) -> Option<(usize, bool)> {
        if before {
            if let Some(next) = next {
                return Some((next, true));
            }
            let mut up = parent;
            while up != root {
                let above = self.parent(up)?;
                let s = self.children(above);
                self.poll_by(s.len()).ok()?;
                let i = s.iter().position(|&c| c == up)?;
                if let Some(&next) = s.get(i + 1) {
                    return Some((next, true));
                }
                up = above;
            }
        }
        let Some(previous) = previous else {
            return Some((parent, false));
        };
        let mut last = previous;
        while let Some(&child) = self.children(last).last() {
            self.poll().ok()?;
            last = child;
        }
        Some((last, false))
    }
}
