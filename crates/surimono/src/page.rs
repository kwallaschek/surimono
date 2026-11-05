//! What [`Renderer::render_page`](crate::Renderer::render_page) gives back,
//! and the parts of a page render that need no script.

use surimono_core::{SettleOptions, SettleOutcome};

/// A page after its scripts ran: HTML to print, and what happened on the way.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Page {
    /// The page as it stood when it settled, with its islands rendered.
    /// See [`Renderer::render_page`](crate::Renderer::render_page).
    pub html: String,
    /// What the scripts did.
    pub report: PageReport,
}

/// How a page's event loop ended. In every case the page is printed as it
/// stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PageSettle {
    /// Nothing was left to run.
    Settled,
    /// Timers were left, due after the virtual time budget
    /// ([`Options::page_budget_ms`](crate::Options::page_budget_ms)), as with
    /// an interval nobody clears. Usual for a page with a ticking clock or a
    /// polling loop.
    BudgetReached,
    /// Tasks were still due at the task limit
    /// ([`Options::page_max_tasks`](crate::Options::page_max_tasks)): the page
    /// may have been stopped part way.
    TaskLimit,
}

impl PageSettle {
    /// A short, stable, kebab-case name, used by the bindings.
    pub fn name(self) -> &'static str {
        match self {
            Self::Settled => "settled",
            Self::BudgetReached => "budget-reached",
            Self::TaskLimit => "task-limit",
        }
    }
}

/// What a page's scripts did. A script's exception doesn't fail a page, as it
/// doesn't in a browser: the page goes on, and it is counted here. Check this
/// report if a page that prints wrong would matter to you.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PageReport {
    /// How the event loop ended.
    pub settle: PageSettle,
    /// Virtual milliseconds that passed before it did.
    pub virtual_ms: u64,
    /// Tasks run: timers, animation frames and the steps of loading the page.
    pub tasks_run: u64,
    /// Timers and animation frames that never ran.
    pub timers_left: usize,
    /// Exceptions no script caught and no `error` listener cancelled.
    pub uncaught: u64,
    /// The first few of those, as messages, each cut to a few KiB.
    pub errors: Vec<String>,
    /// Promises rejected with no handler.
    pub unhandled_rejections: u64,
    /// The first few reasons, as messages, each cut to a few KiB.
    pub rejections: Vec<String>,
    /// `src` of each external script that couldn't be had; the page ran
    /// without them, and each one's element got an `error` event.
    pub missing_scripts: Vec<String>,
}

impl PageReport {
    /// Whether nothing went wrong: the page settled with no exception, no
    /// rejected promise and every script found. A page stopped by the budget
    /// counts as clean, because its work is done or will never be.
    pub fn is_clean(&self) -> bool {
        self.settle != PageSettle::TaskLimit
            && self.uncaught == 0
            && self.unhandled_rejections == 0
            && self.missing_scripts.is_empty()
    }

    /// The facade's report for the DOM crate's.
    pub(crate) fn new(from: surimono_dom::PageReport) -> Result<Self, String> {
        let settle = match from.settle.outcome {
            SettleOutcome::Settled if from.settle.timers_left > 0 => PageSettle::BudgetReached,
            SettleOutcome::Settled => PageSettle::Settled,
            SettleOutcome::TaskLimit => PageSettle::TaskLimit,
            // Nothing opens a wait, so this is a bug, not a page's doing.
            SettleOutcome::WaitingExternal => {
                return Err("the page stopped on a wait nothing opened".to_owned());
            }
        };
        Ok(Self {
            settle,
            virtual_ms: from.settle.virtual_now_ms,
            tasks_run: from.settle.tasks_run,
            timers_left: from.settle.timers_left,
            uncaught: from.uncaught,
            errors: from.errors,
            unhandled_rejections: from.unhandled_rejections,
            rejections: from.rejections,
            missing_scripts: from.missing_scripts,
        })
    }
}

/// The settle rule for a page, from the options.
pub(crate) fn settle_options(budget_ms: Option<u64>, max_tasks: Option<u64>) -> SettleOptions {
    let default = SettleOptions::default();
    SettleOptions {
        virtual_budget_ms: budget_ms.unwrap_or(default.virtual_budget_ms),
        max_tasks: max_tasks.unwrap_or(default.max_tasks),
    }
}

/// The doctype of `html` as written, if it has one where the HTML parser
/// takes one ("the initial insertion mode", HTML §13.2.6.4.1): after a byte
/// order mark, white space and comments.
///
/// The DOM keeps no doctype, and a page printed without the one it came with
/// is laid out in quirks mode. It ends at the first `>`, quotes or not, as
/// the tokenizer ends it.
pub(crate) fn doctype(html: &str) -> Option<&str> {
    let mut rest = html.strip_prefix('\u{feff}').unwrap_or(html);
    loop {
        rest = rest.trim_start_matches(['\t', '\n', '\u{c}', '\r', ' ']);
        let bytes = rest.as_bytes();
        if bytes.len() >= 9 && bytes[..9].eq_ignore_ascii_case(b"<!doctype") {
            return rest.find('>').map(|end| &rest[..=end]);
        }
        if let Some(body) = rest.strip_prefix("<!--") {
            rest = after_comment(body)?;
        } else if rest.starts_with("<!") || rest.starts_with("<?") || rest.starts_with("</") {
            // A bogus comment, up to the first `>`. (`</` followed by a
            // letter is an end tag, which this mode also ignores... but it
            // ends the mode with a quirks document, so no doctype follows.)
            if rest.starts_with("</") && bytes.get(2).is_some_and(u8::is_ascii_alphabetic) {
                return None;
            }
            rest = &rest[rest.find('>')? + 1..];
        } else {
            return None;
        }
    }
}

/// What follows a comment whose text starts at `body`, as the tokenizer ends
/// a comment: `-->`, `--!>`, or at once with `>` or `->`.
fn after_comment(body: &str) -> Option<&str> {
    if let Some(after) = body.strip_prefix('>').or_else(|| body.strip_prefix("->")) {
        return Some(after);
    }
    let plain = body.find("-->").map(|i| i + 3);
    let bang = body.find("--!>").map(|i| i + 4);
    let end = match (plain, bang) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return None,
    };
    Some(&body[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_doctype_is_found_where_the_parser_takes_it() {
        for (html, want) in [
            ("<!doctype html><p>", Some("<!doctype html>")),
            ("\u{feff} \n<!DOCTYPE html>", Some("<!DOCTYPE html>")),
            ("<!-- a -->\n<!DocType html>x", Some("<!DocType html>")),
            ("<!---->\n<!doctype html>", Some("<!doctype html>")),
            ("<!-->\n<!doctype html>", Some("<!doctype html>")),
            ("<!--->\n<!doctype html>", Some("<!doctype html>")),
            ("<!-- a --!><!doctype html>", Some("<!doctype html>")),
            (
                "<?xml version='1.0'?><!doctype html>",
                Some("<!doctype html>"),
            ),
            ("<!bogus><!doctype html>", Some("<!doctype html>")),
            (
                "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\">",
                Some("<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\">"),
            ),
            // The tokenizer ends it at the first `>`, in quotes or not.
            ("<!doctype a\">\"><p>", Some("<!doctype a\">")),
            ("<p>x</p><!doctype html>", None),
            ("x<!doctype html>", None),
            ("</p><!doctype html>", None),
            ("<!-- never closed <!doctype html>", None),
            ("<!doctype html", None),
            ("", None),
        ] {
            assert_eq!(doctype(html), want, "{html:?}");
        }
    }

    #[test]
    fn a_clean_report() {
        let report = PageReport {
            settle: PageSettle::Settled,
            virtual_ms: 0,
            tasks_run: 6,
            timers_left: 0,
            uncaught: 0,
            errors: vec![],
            unhandled_rejections: 0,
            rejections: vec![],
            missing_scripts: vec![],
        };
        assert!(report.is_clean());
        let budget = PageReport {
            settle: PageSettle::BudgetReached,
            ..report.clone()
        };
        assert!(budget.is_clean());
        for dirty in [
            PageReport {
                settle: PageSettle::TaskLimit,
                ..report.clone()
            },
            PageReport {
                uncaught: 1,
                ..report.clone()
            },
            PageReport {
                unhandled_rejections: 1,
                ..report.clone()
            },
            PageReport {
                missing_scripts: vec!["a.js".into()],
                ..report
            },
        ] {
            assert!(!dirty.is_clean(), "{dirty:?}");
        }
    }
}
