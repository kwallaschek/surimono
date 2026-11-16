//! HTML §4.12.1.1 for script elements: the state blitz-dom has no room for,
//! which scripts the DOM's changes make ready to prepare, and "prepare the
//! script element" itself. The page's parsed scripts run from the lifecycle
//! in `glue_tail.js`, which takes them from [`Document::scripts`]; a script a
//! script inserts is prepared here, when the operation that connected it (or
//! changed its children or `src`) returns: see "A script a script inserts" in
//! DESIGN.md.
//!
//! Both paths ask [`Document::script_kind`] what a script element is, the one
//! place that decides it, so a later kind (module scripts, P2-RT-12) changes
//! one function.

use html5ever::{QualName, ns};

use crate::document::{ASCII_WHITESPACE, Document, is_javascript_mime_essence};

/// `already started`.
const STARTED: u8 = 1;
/// Parser-inserted: the element's parser document isn't null.
const PARSER: u8 = 2;
/// `force async` is false. It starts true for an element a script makes.
const NOT_FORCE_ASYNC: u8 = 4;
/// Still open at the end of the page's input: the parser marked it started
/// (HTML §13.2.6.4.8, an end-of-file token in the "text" mode), so the
/// lifecycle doesn't run it either.
const UNCLOSED: u8 = 8;

/// A script element's type, as "prepare the script element" works it out
/// from `type` and `language`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptKind {
    Classic,
    /// Doesn't run here (P2-RT-12 is the plan), with no error, as before.
    Module,
    /// Neither registered nor an error here.
    ImportMap,
    /// Neither registered nor an error here.
    SpeculationRules,
}

/// What "prepare the script element" left to do for a script that isn't
/// parser-inserted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Prepared {
    /// Nothing runs: started already, empty, not connected, a type that
    /// doesn't run here, or in a document with no browsing context.
    Nothing,
    /// An inline classic script, to run now ("immediately execute").
    Inline(String),
    /// A classic script to fetch, its `src` as written.
    Fetch(String),
    /// `src` is empty: an `error` event at the element, in a task.
    Error,
}

impl Document {
    /// The type "prepare the script element" gives a script element (HTML
    /// §4.12.1.1, steps 9 to 11), or `None` for one that doesn't run at all.
    pub(crate) fn script_kind(&self, id: usize) -> Option<ScriptKind> {
        let ty = self.attribute(id, "type");
        let language = self.attribute(id, "language");
        let block = match (ty.as_deref(), language.as_deref()) {
            (Some(""), _) | (None, None | Some("")) => "text/javascript".to_owned(),
            (Some(ty), _) => ty.to_owned(),
            (None, Some(language)) => format!("text/{language}"),
        };
        if is_javascript_mime_essence(&block)
            || ty.is_some_and(|t| is_javascript_mime_essence(t.trim_matches(ASCII_WHITESPACE)))
        {
            return Some(ScriptKind::Classic);
        }
        [
            ("module", ScriptKind::Module),
            ("importmap", ScriptKind::ImportMap),
            ("speculationrules", ScriptKind::SpeculationRules),
        ]
        .into_iter()
        .find(|(name, _)| block.eq_ignore_ascii_case(name))
        .map(|(_, kind)| kind)
    }

    /// HTML §4.12.1.1 step 20: a classic script with `event` and `for` runs
    /// only for the window's `onload`.
    pub(crate) fn script_event_for_blocks(&self, id: usize) -> bool {
        let (Some(event), Some(target)) = (self.attribute(id, "event"), self.attribute(id, "for"))
        else {
            return false;
        };
        let event = event.trim_matches(ASCII_WHITESPACE);
        !(target
            .trim_matches(ASCII_WHITESPACE)
            .eq_ignore_ascii_case("window")
            && (event.eq_ignore_ascii_case("onload") || event.eq_ignore_ascii_case("onload()")))
    }

    fn is_script(&self, id: usize) -> bool {
        self.element_name(id)
            .is_some_and(|q| q.ns == ns!(html) && q.local.as_ref() == "script")
    }

    fn script_flags(&self, id: usize) -> u8 {
        self.script_states.get(&id).copied().unwrap_or(0)
    }

    fn set_script_flags(&mut self, id: usize, flags: u8) {
        if flags == 0 {
            self.script_states.remove(&id);
        } else {
            self.script_states.insert(id, flags);
        }
    }

    /// The script elements under `top`, in tree order, `top` included; with
    /// template contents if `templates`.
    fn scripts_under(&self, top: usize, templates: bool) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![top];
        while let Some(n) = stack.pop() {
            if self.is_script(n) {
                out.push(n);
            }
            if templates && let Some(content) = self.template_of(n) {
                stack.push(content);
            }
            stack.extend(self.children(n).iter().rev().copied());
        }
        out
    }

    /// After a whole document is parsed, what the parser's own "prepare the
    /// script element" leaves on each script it made in the tree: one that
    /// would run (or does, from the page's lifecycle) is parser-inserted and
    /// started; an empty one, or one of a type that doesn't run, was given
    /// back (its parser document is null) and runs if a script changes it so
    /// that it can. Template contents are left alone: never connected, never
    /// prepared, so their scripts run once a script inserts them.
    pub(crate) fn mark_parsed_scripts(&mut self, top: usize) {
        for id in self.scripts_under(top, false) {
            let ready = (self.attribute(id, "src").is_some() || !self.child_text(id).is_empty())
                && self.script_kind(id).is_some();
            let flags = if ready {
                STARTED | PARSER | NOT_FORCE_ASYNC
            } else if self.attribute(id, "async").is_some() {
                NOT_FORCE_ASYNC
            } else {
                0
            };
            self.set_script_flags(id, flags);
        }
    }

    /// The scripts the page's parser left open at the end of its input,
    /// which it marked started: none of them runs, now or once changed.
    /// Chromium 153 doesn't run one either, but runs it once a script
    /// appends a child to it; the spec wins (DESIGN.md).
    pub(crate) fn mark_unclosed_scripts(&mut self, marked: &[usize]) {
        for &id in marked {
            if self.is_script(id) {
                let flags = self.script_flags(id) | STARTED | PARSER | NOT_FORCE_ASYNC | UNCLOSED;
                self.set_script_flags(id, flags);
            }
        }
    }

    /// Whether the page's parser left script `id` open at the end.
    pub(crate) fn script_unclosed(&self, id: usize) -> bool {
        self.script_flags(id) & UNCLOSED != 0
    }

    /// After a fragment parse (`innerHTML`, `outerHTML`, `insertAdjacentHTML`):
    /// the fragment parser marks every script it makes started (HTML §13.2.6.4.4),
    /// so none of them ever runs. For `createContextualFragment`
    /// (`contextual`), HTML §8.5 then unmarks them as started and as
    /// parser-inserted, so they run once inserted; `force async` stays false.
    pub(crate) fn mark_fragment_scripts(&mut self, fragment: usize, contextual: bool) {
        // The parse's own, for the contextual call that follows it.
        let unclosed = if contextual {
            std::mem::take(&mut self.unclosed_scripts)
        } else {
            Vec::new()
        };
        for id in self.scripts_under(fragment, !contextual) {
            // One the input left open was marked at the end of it, and stays
            // so (WPT domparsing/tentative/unclosed-script-already-started).
            let flags = if contextual && !unclosed.contains(&id) {
                NOT_FORCE_ASYNC
            } else {
                STARTED | PARSER | NOT_FORCE_ASYNC
            };
            self.set_script_flags(id, flags);
        }
    }

    /// The cloning steps (HTML §4.12.1): the copy is started if `from` is.
    /// Appending the copy's attributes counts as adding them, so an `async`
    /// one makes `force async` false.
    pub(crate) fn clone_script_state(&mut self, from: usize, copy: usize) {
        if !self.is_script(from) {
            return;
        }
        let mut flags = self.script_flags(from) & STARTED;
        if self.attribute(copy, "async").is_some() {
            flags |= NOT_FORCE_ASYNC;
        }
        self.set_script_flags(copy, flags);
    }

    /// Forgets a node that is dropped: its id may be used again.
    pub(crate) fn drop_script_state(&mut self, id: usize) {
        if !self.script_states.is_empty() {
            self.script_states.remove(&id);
        }
    }

    /// `script.async` (HTML §4.12.1).
    pub(crate) fn script_async(&self, id: usize) -> bool {
        self.script_flags(id) & NOT_FORCE_ASYNC == 0 || self.attribute(id, "async").is_some()
    }

    /// The `async` setter's first step; the attribute is the caller's.
    pub(crate) fn clear_force_async(&mut self, id: usize) {
        let flags = self.script_flags(id) | NOT_FORCE_ASYNC;
        self.set_script_flags(id, flags);
    }

    /// DOM §4.2.3 "insert", for `nodes` now under `parent`: the children
    /// changed steps of a script `parent`, then the post-connection steps of
    /// each script among the inserted nodes and their descendants, in tree
    /// order. Queued, and run when the operation returns.
    pub(crate) fn scripts_inserted(&mut self, parent: usize, nodes: &[usize]) {
        self.script_children_changed(parent);
        if !self.is_connected(parent) {
            return;
        }
        for &n in nodes {
            // Not template contents: those aren't connected.
            let found = self.scripts_under(n, false);
            self.to_prepare.extend(found);
        }
    }

    /// The script children changed steps, for an insertion. DOM's text runs
    /// them for a removal and "replace data" too; WPT
    /// `script-does-not-run-on-child-removal` and Chromium 153 don't, and we
    /// follow them (DESIGN.md).
    pub(crate) fn script_children_changed(&mut self, parent: usize) {
        if self.is_script(parent)
            && self.script_flags(parent) & STARTED == 0
            && self.to_prepare.last() != Some(&parent)
            && self.is_connected(parent)
        {
            self.to_prepare.push(parent);
        }
    }

    /// The script element's attribute change steps: `src` set on a
    /// connected one, and `async` added.
    pub(crate) fn script_attribute_changed(&mut self, id: usize, name: &QualName, set: bool) {
        if !name.ns.is_empty() || !self.is_script(id) {
            return;
        }
        match name.local.as_ref() {
            "src" if set && self.is_connected(id) => self.to_prepare.push(id),
            "async" if set => self.clear_force_async(id),
            _ => {}
        }
    }

    /// Script elements queued for their post-connection steps, in order, and
    /// none left: each operation prepares its own, so a script that runs in
    /// the middle of them prepares the ones it queues first.
    pub(crate) fn take_scripts_to_prepare(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.to_prepare)
    }

    /// Forgets the queue, before an operation starts: what a parse or a
    /// clone queued outside one is never prepared.
    pub(crate) fn clear_scripts_to_prepare(&mut self) {
        self.to_prepare.clear();
    }

    /// The script HTML element post-connection steps and "prepare the script
    /// element" (HTML §4.12.1.1) for script `id`, which a script inserted,
    /// changed the children of, or gave a `src`. Called from the glue for
    /// each queued element, when the operation that queued it returns.
    ///
    /// Departures, each in DESIGN.md: `nomodule` is ignored, as for the
    /// parser's scripts; module scripts, import maps and speculation rules are
    /// started and do nothing; `src` is the attribute as written, with no
    /// URL parsing; there is no `charset`, CORS, integrity or CSP.
    pub(crate) fn prepare_script(&mut self, id: usize) -> Prepared {
        if !self.is_script(id) {
            return Prepared::Nothing;
        }
        let flags = self.script_flags(id);
        // Post-connection steps, step 1; and prepare's step 1.
        if flags & (PARSER | STARTED) != 0 || !self.is_connected(id) {
            return Prepared::Nothing;
        }
        let src = self.attribute(id, "src");
        let source = self.child_text(id);
        if src.is_none() && source.is_empty() {
            return Prepared::Nothing;
        }
        let Some(kind) = self.script_kind(id) else {
            return Prepared::Nothing;
        };
        self.set_script_flags(id, flags | STARTED);
        // Scripting is disabled for a document with no browsing context:
        // only the page's has one (frames run no scripts here).
        if self.owner_of_node(id) != self.root() || kind != ScriptKind::Classic {
            return Prepared::Nothing;
        }
        if self.script_event_for_blocks(id) {
            return Prepared::Nothing;
        }
        match src {
            Some(src) if src.is_empty() => Prepared::Error,
            Some(src) => Prepared::Fetch(src),
            None => Prepared::Inline(source),
        }
    }
}
