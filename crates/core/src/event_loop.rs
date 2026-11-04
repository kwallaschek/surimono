//! The event loop on a virtual clock, and the settle rule.
//!
//! Internal phase-2 scaffolding (`#[doc(hidden)]`): `surimono-dom` turns it
//! on with [`SandboxOptions::with_event_loop`](crate::SandboxOptions) and
//! drives it with [`Sandbox::run_until_settled`](crate::Sandbox); nothing in
//! the facade, the CLI or the gem uses it, and islands don't turn it on.
//!
//! # What a script sees
//!
//! `setTimeout`, `setInterval`, `clearTimeout`, `clearInterval`,
//! `queueMicrotask`, `requestAnimationFrame` and `cancelAnimationFrame`
//! (defined in `prelude.js`), behaving as the HTML standard says except that
//! no real time passes: the clock jumps to the next timer's due time.
//! `Date` and `performance.now()` read that clock, so they stand still
//! during a task and move only when the next one starts.
//!
//! # Where things live
//!
//! Rust holds the queue: ids, due times and nesting levels ([`LoopState`]),
//! never a JavaScript value. Each timer's handler and arguments sit in a
//! `Map` private to the prelude, keyed by id, so the memory limit covers
//! them and they go with the context at teardown. A timer's Rust entry is
//! outside the JavaScript heap, so live timers are capped too
//! ([`MAX_LIVE_TIMERS`]); passing the cap fails the call with
//! [`SandboxError::MemoryLimit`]. A script can't reach the queue: the
//! natives and `fire` stay in the prelude's closure.
//!
//! # The settle rule
//!
//! [`run`] loops: drain the microtasks, take the first timer by (due time,
//! sequence), run it, and again. It stops between tasks, with the microtasks
//! drained, as soon as one of these holds:
//!
//! - no timer is due within the budget and no external wait is pending:
//!   [`SettleOutcome::Settled`] (`timers_left > 0` means "settled by
//!   budget": a timer exists but is due later, as with an interval nobody
//!   clears);
//! - no timer is due within the budget and an external wait is pending:
//!   [`SettleOutcome::WaitingExternal`]. The caller resolves its waits and
//!   calls again;
//! - a timer is due but [`SettleOptions::max_tasks`] tasks have run:
//!   [`SettleOutcome::TaskLimit`].
//!
//! The virtual budget and the task count are measured from the start of the
//! context, not of the call, so calling again can't extend them. The real
//! time, memory and stack limits apply to the whole of a
//! `run_until_settled` call, which is one call to the sandbox: every
//! callback, microtask and check between them runs under the same deadline.
//! A callback that throws is reported ([`SettleReport::errors`]) and the
//! loop goes on, as in a browser; hitting a limit ends the call with the
//! typed error.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use rquickjs::{Ctx, Exception, Function, Object, Value};

use crate::SandboxError;
use crate::engine::LimitState;
use crate::sandbox::{classify, drain_jobs, exception_error, guard_native};

/// Most timers alive at once. Their Rust entries aren't in the JavaScript
/// heap, so this is what bounds them; far above what real pages use.
pub(crate) const MAX_LIVE_TIMERS: usize = 100_000;

/// Most uncaught exceptions kept for [`SettleReport::errors`]; the rest are
/// only counted.
const MAX_KEPT_ERRORS: usize = 8;

/// HTML timers: a timer set at a nesting level above this...
const CLAMP_NESTING: u32 = 5;
/// ...waits at least this many milliseconds.
const CLAMP_MS: u32 = 4;

/// Virtual milliseconds between animation frames (a 60 Hz display, in the
/// clock's whole milliseconds).
const FRAME_MS: u64 = 16;

/// Largest virtual budget: what a JavaScript number holds exactly.
const MAX_BUDGET_MS: u64 = (1 << 53) - 1;

/// When [`Sandbox::run_until_settled`](crate::Sandbox) gives up on a page
/// that keeps scheduling work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettleOptions {
    /// Timers due later than this many virtual milliseconds after the start
    /// of the context don't run; one due exactly then does.
    pub virtual_budget_ms: u64,
    /// Most tasks (timer and animation-frame callbacks) to run in the life
    /// of the context. Microtasks don't count; the time limit stops those.
    pub max_tasks: u64,
}

impl SettleOptions {
    /// Default virtual budget: 10 seconds.
    pub const DEFAULT_VIRTUAL_BUDGET_MS: u64 = 10_000;
    /// Default task limit: 100,000.
    pub const DEFAULT_MAX_TASKS: u64 = 100_000;
}

impl Default for SettleOptions {
    fn default() -> Self {
        Self {
            virtual_budget_ms: Self::DEFAULT_VIRTUAL_BUDGET_MS,
            max_tasks: Self::DEFAULT_MAX_TASKS,
        }
    }
}

/// Why [`Sandbox::run_until_settled`](crate::Sandbox) stopped. In every case
/// the microtasks are drained and script state is consistent, so the page
/// can be serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleOutcome {
    /// Nothing is left to do within the budget.
    Settled,
    /// Timers were still due when the task limit was reached.
    TaskLimit,
    /// Only external waits are left; resolve them and call again.
    WaitingExternal,
}

/// What [`Sandbox::run_until_settled`](crate::Sandbox) found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettleReport {
    /// Why it stopped.
    pub outcome: SettleOutcome,
    /// The virtual clock, in milliseconds since the start of the context:
    /// the due time of the last task run.
    pub virtual_now_ms: u64,
    /// Tasks run so far in the life of the context.
    pub tasks_run: u64,
    /// Timers that haven't run yet.
    pub timers_left: usize,
    /// External waits still pending.
    pub external_pending: usize,
    /// Exceptions no script caught since the last report, in callbacks of
    /// timers, frames and `queueMicrotask`.
    pub uncaught: u64,
    /// The first few of those.
    pub errors: Vec<SandboxError>,
}

impl SettleReport {
    /// Whether the page settled only because what's left is due after the
    /// budget.
    pub fn settled_by_budget(&self) -> bool {
        self.outcome == SettleOutcome::Settled && self.timers_left > 0
    }
}

/// A host's handle on a sandbox's event loop, for external waits: resources
/// the page is waiting on (fonts, images) that the settle rule must see.
///
/// It belongs to the sandbox's current runtime: take a new one after a
/// reset or a limit error (an old one counts on a loop that is gone).
#[derive(Debug, Clone)]
pub struct EventLoopHandle {
    pub(crate) shared: Shared,
}

impl EventLoopHandle {
    /// Records that the page now waits on one more external thing.
    pub fn external_begin(&self) {
        let mut st = self.shared.borrow_mut();
        st.external = st.external.saturating_add(1);
    }

    /// Records that one external wait ended. Extra calls are ignored.
    pub fn external_end(&self) {
        let mut st = self.shared.borrow_mut();
        st.external = st.external.saturating_sub(1);
    }

    /// External waits pending now.
    pub fn external_pending(&self) -> usize {
        self.shared.borrow().external
    }

    /// The virtual clock, in milliseconds since the start of the context.
    pub fn virtual_now_ms(&self) -> u64 {
        self.shared.borrow().now
    }
}

/// What a timer does when it comes due. The numbers are the `kind` argument
/// of the `schedule` native (see `prelude.js`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Timeout = 0,
    Interval = 1,
    Frame = 2,
}

impl Kind {
    fn from_js(kind: u32) -> Self {
        match kind {
            0 => Self::Timeout,
            1 => Self::Interval,
            _ => Self::Frame,
        }
    }
}

/// Queue order: due time, then the order timers were set (the HTML
/// standard's "run steps after a timeout" gives the same order for timers
/// of one ordering identifier).
type Key = (u64, u64);

/// One queued run of a timer.
#[derive(Debug, Clone, Copy)]
struct Task {
    id: i32,
    kind: Kind,
    /// As asked for, after the "less than 0" step and before the clamp; an
    /// interval is set again from this.
    timeout: u32,
    /// The task's timer nesting level.
    nesting: u32,
}

/// A live id: set and not cleared, or running right now.
#[derive(Debug, Clone, Copy)]
struct Slot {
    /// Where it is in the queue; `None` while its callback runs.
    key: Option<Key>,
    frame: bool,
}

/// The loop's state. Holds no JavaScript value, so it can outlive the
/// runtime in any order.
#[derive(Debug, Default)]
pub(crate) struct LoopState {
    queue: BTreeMap<Key, Task>,
    /// Every live id. Lookups only: nothing iterates this, or the queue
    /// except from the front, so order is always (due, seq).
    live: BTreeMap<i32, Slot>,
    /// Ids count up from 1 and are never reused; animation frames share
    /// them, which no script can tell from separate counters except by
    /// comparing the numbers.
    last_id: i32,
    next_seq: u64,
    /// Virtual milliseconds since the start of the context.
    now: u64,
    /// Nesting level of the running timer task; 0 outside one (including
    /// during its microtasks, each a task of its own in the standard).
    nesting: u32,
    tasks_run: u64,
    external: usize,
    uncaught: u64,
    errors: Vec<SandboxError>,
    /// A limit error found in a callback's report: the call must end.
    fatal: Option<SandboxError>,
}

pub(crate) type Shared = Rc<RefCell<LoopState>>;

/// What to do next.
enum Step {
    Run(Task, u64),
    Stop(SettleOutcome),
}

impl LoopState {
    /// The timer initialization steps from the nesting level on (HTML
    /// §8.7): clamps, picks the time and queues the task. `id` is given
    /// when an interval goes round again.
    fn arm(&mut self, kind: Kind, timeout: u32, nesting: u32, id: i32) {
        let due = match kind {
            Kind::Frame => (self.now / FRAME_MS + 1) * FRAME_MS,
            Kind::Timeout | Kind::Interval => {
                let wait = if nesting > CLAMP_NESTING {
                    timeout.max(CLAMP_MS)
                } else {
                    timeout
                };
                self.now + u64::from(wait)
            }
        };
        let key = (due, self.next_seq);
        self.next_seq += 1;
        self.queue.insert(
            key,
            Task {
                id,
                kind,
                timeout,
                nesting: nesting.saturating_add(1),
            },
        );
        self.live.insert(
            id,
            Slot {
                key: Some(key),
                frame: kind == Kind::Frame,
            },
        );
    }

    /// `setTimeout` and friends. `None` when out of ids or over the cap.
    fn schedule(&mut self, kind: Kind, timeout: i32) -> Option<i32> {
        if self.live.len() >= MAX_LIVE_TIMERS || self.last_id == i32::MAX {
            return None;
        }
        self.last_id += 1;
        let id = self.last_id;
        // "If timeout is less than 0, then set timeout to 0."
        let timeout = u32::try_from(timeout).unwrap_or(0);
        // A frame callback isn't a task of the timer algorithm, so only
        // timers read the running task's level.
        self.arm(kind, timeout, self.nesting, id);
        Some(id)
    }

    /// `clearTimeout` and friends: true if `id` was live. Timers and frames
    /// clear only their own kind.
    fn cancel(&mut self, id: i32, frame: bool) -> bool {
        match self.live.get(&id) {
            Some(slot) if slot.frame == frame => {}
            _ => return false,
        }
        if let Some(Slot { key: Some(key), .. }) = self.live.remove(&id) {
            self.queue.remove(&key);
        }
        true
    }

    /// Applies the settle rule: the next task, or why there is none.
    fn step(&mut self, options: &SettleOptions) -> Step {
        let Some(entry) = self
            .queue
            .first_entry()
            .filter(|e| e.key().0 <= options.virtual_budget_ms)
        else {
            return Step::Stop(if self.external > 0 {
                SettleOutcome::WaitingExternal
            } else {
                SettleOutcome::Settled
            });
        };
        if self.tasks_run >= options.max_tasks {
            return Step::Stop(SettleOutcome::TaskLimit);
        }
        let ((due, _), task) = entry.remove_entry();
        self.now = due;
        self.tasks_run += 1;
        if let Some(slot) = self.live.get_mut(&task.id) {
            slot.key = None;
        }
        self.nesting = match task.kind {
            Kind::Frame => 0,
            Kind::Timeout | Kind::Interval => task.nesting,
        };
        Step::Run(task, due)
    }

    /// The end of a task, after its microtasks: an interval still live goes
    /// round again from the task's own nesting level; anything else is gone.
    ///
    /// Read literally, the standard's microtask checkpoint leaves "currently
    /// running task" null, which would restart the level of an interval
    /// that queued a microtask. Its note says the level covers repeating
    /// timers, so the task's own level is kept; `setInterval(f, 0)` with a
    /// promise inside still moves the clock.
    fn finish(&mut self, task: Task) {
        if !self.live.contains_key(&task.id) {
            return;
        }
        if task.kind == Kind::Interval {
            self.arm(task.kind, task.timeout, task.nesting, task.id);
        } else {
            self.live.remove(&task.id);
        }
    }

    /// Records an exception no script caught; a limit error ends the call.
    fn note(&mut self, error: SandboxError) {
        if error.is_limit() {
            self.fatal.get_or_insert(error);
            return;
        }
        self.uncaught += 1;
        if self.errors.len() < MAX_KEPT_ERRORS {
            self.errors.push(error);
        }
    }

    fn report(&mut self, outcome: SettleOutcome) -> SettleReport {
        let errors = std::mem::take(&mut self.errors);
        SettleReport {
            outcome,
            virtual_now_ms: self.now,
            tasks_run: self.tasks_run,
            timers_left: self.queue.len(),
            external_pending: self.external,
            uncaught: std::mem::take(&mut self.uncaught),
            errors,
        }
    }
}

/// Checks `options` are in range.
pub(crate) fn validate(options: &SettleOptions) -> Result<(), SandboxError> {
    if options.virtual_budget_ms > MAX_BUDGET_MS {
        return Err(SandboxError::InvalidOptions {
            message: format!(
                "virtual_budget_ms is {}, it must be at most {MAX_BUDGET_MS}",
                options.virtual_budget_ms
            ),
        });
    }
    Ok(())
}

/// Builds the natives the prelude's event loop calls.
pub(crate) fn host_object<'js>(
    ctx: &Ctx<'js>,
    shared: &Shared,
    limit: &std::sync::Arc<LimitState>,
) -> rquickjs::Result<Object<'js>> {
    let host = Object::new(ctx.clone())?;
    let (st, stop) = (shared.clone(), limit.clone());
    host.set(
        "schedule",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, kind: u32, timeout: i32| -> rquickjs::Result<i32> {
                guard_native(&stop, &ctx, || {
                    let id = st.borrow_mut().schedule(Kind::from_js(kind), timeout);
                    id.ok_or_else(|| {
                        // Sticky, like a refused allocation: catching this
                        // doesn't save the call.
                        stop.exceed_memory();
                        Exception::throw_internal(&ctx, "too many timers")
                    })
                })
            },
        )?,
    )?;
    let (st, stop) = (shared.clone(), limit.clone());
    host.set(
        "cancel",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, id: i32, frame: bool| {
            guard_native(&stop, &ctx, || Ok(st.borrow_mut().cancel(id, frame)))
        })?,
    )?;
    let (st, stop) = (shared.clone(), limit.clone());
    host.set(
        "report",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, thrown: Value<'js>| {
            guard_native(&stop, &ctx, || {
                // Reading the exception can run its getters, so no borrow yet.
                let error = exception_error(thrown);
                let _ = ctx.catch();
                st.borrow_mut().note(error);
                Ok(())
            })
        })?,
    )?;
    Ok(host)
}

/// Drains the microtasks: an exception escaping a job is reported and the
/// rest still run; a limit ends the call.
fn drain<'js>(ctx: &Ctx<'js>, limit: &LimitState, shared: &Shared) -> Result<(), SandboxError> {
    shared.borrow_mut().nesting = 0;
    while let Err(e) = drain_jobs(ctx, limit) {
        survive(ctx, limit, shared, e)?;
    }
    match shared.borrow_mut().fatal.take() {
        Some(fatal) => Err(fatal),
        None => Ok(()),
    }
}

/// Reports an exception a callback let escape, or fails if it was a limit
/// or anything else that isn't a script's own exception.
fn survive<'js>(
    ctx: &Ctx<'js>,
    limit: &LimitState,
    shared: &Shared,
    e: rquickjs::Error,
) -> Result<(), SandboxError> {
    let error = classify(ctx, limit, e);
    // Reading a hostile exception's properties can leave another pending.
    let _ = ctx.catch();
    match error {
        SandboxError::Exception { .. } => {
            shared.borrow_mut().note(error);
            Ok(())
        }
        other => Err(other),
    }
}

/// The loop itself; see the module docs. `fire` is the prelude's dispatcher.
pub(crate) fn run<'js>(
    ctx: &Ctx<'js>,
    limit: &LimitState,
    shared: &Shared,
    fire: Function<'js>,
    options: &SettleOptions,
) -> Result<SettleReport, SandboxError> {
    loop {
        drain(ctx, limit, shared)?;
        let step = shared.borrow_mut().step(options);
        let (task, due) = match step {
            Step::Run(task, due) => (task, due),
            Step::Stop(outcome) => return Ok(shared.borrow_mut().report(outcome)),
        };
        // The borrow has ended: the callback calls the natives. The clock
        // fits a JavaScript number: `validate` bounds the budget.
        let frame = task.kind == Kind::Frame;
        if let Err(e) = fire.call::<_, ()>((task.id, due as f64, frame)) {
            survive(ctx, limit, shared, e)?;
        }
        drain(ctx, limit, shared)?;
        shared.borrow_mut().finish(task);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_order(st: &mut LoopState) -> Vec<(u64, i32)> {
        let options = SettleOptions::default();
        let mut order = Vec::new();
        while let Step::Run(task, due) = st.step(&options) {
            order.push((due, task.id));
            st.finish(task);
        }
        order
    }

    #[test]
    fn tasks_run_by_due_time_then_by_sequence() {
        let mut st = LoopState::default();
        let a = st.schedule(Kind::Timeout, 10).unwrap();
        let b = st.schedule(Kind::Timeout, 5).unwrap();
        let c = st.schedule(Kind::Timeout, 10).unwrap();
        let d = st.schedule(Kind::Timeout, 0).unwrap();
        assert_eq!(run_order(&mut st), [(0, d), (5, b), (10, a), (10, c)]);
    }

    #[test]
    fn cancelling_a_queued_timer_frees_both_entries() {
        let mut st = LoopState::default();
        let id = st.schedule(Kind::Timeout, 10).unwrap();
        assert!(st.cancel(id, false));
        assert!(!st.cancel(id, false));
        assert!(st.queue.is_empty() && st.live.is_empty());
    }

    #[test]
    fn timers_and_frames_clear_only_their_own_kind() {
        let mut st = LoopState::default();
        let timer = st.schedule(Kind::Timeout, 10).unwrap();
        let frame = st.schedule(Kind::Frame, 0).unwrap();
        assert!(!st.cancel(timer, true));
        assert!(!st.cancel(frame, false));
        assert!(st.cancel(frame, true));
        assert!(st.cancel(timer, false));
    }

    #[test]
    fn the_id_space_ends_in_a_refusal() {
        let mut st = LoopState {
            last_id: i32::MAX,
            ..LoopState::default()
        };
        assert_eq!(st.schedule(Kind::Timeout, 0), None);
    }

    #[test]
    fn a_cleared_running_interval_is_not_set_again() {
        let mut st = LoopState::default();
        let id = st.schedule(Kind::Interval, 1).unwrap();
        let Step::Run(task, _) = st.step(&SettleOptions::default()) else {
            panic!("the interval is due");
        };
        assert!(st.cancel(id, false));
        st.finish(task);
        assert!(st.queue.is_empty() && st.live.is_empty());
    }

    #[test]
    fn budgets_above_a_safe_integer_are_refused() {
        let bad = SettleOptions {
            virtual_budget_ms: u64::MAX,
            ..SettleOptions::default()
        };
        assert!(matches!(
            validate(&bad),
            Err(SandboxError::InvalidOptions { .. })
        ));
        assert_eq!(validate(&SettleOptions::default()), Ok(()));
    }
}
