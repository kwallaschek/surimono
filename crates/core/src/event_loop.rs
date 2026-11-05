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
//! during a task and move only when the next one starts. All of an animation
//! frame's callbacks run as one group, ahead of timers due on the same tick.
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
//! [`run`] loops: drain the microtasks, take the first task by (due time,
//! frames before timers, sequence), run it, and again. It stops between
//! tasks, with the microtasks drained, as soon as one of these holds:
//!
//! - an external wait is pending and the next task would move the clock
//!   (or there is none): [`SettleOutcome::WaitingExternal`]. The caller
//!   resolves its waits and calls again. External work takes no virtual
//!   time, so a pending image or font finishes before any later timer,
//!   whatever real time it took, and a fallback `setTimeout(f, 3000)` can't
//!   overtake it. Tasks due at the current time still run;
//! - no timer is due within the budget: [`SettleOutcome::Settled`]
//!   (`timers_left > 0` means "settled by budget": a timer exists but is
//!   due later, as with an interval nobody clears);
//! - a task is due but [`SettleOptions::max_tasks`] tasks have run:
//!   [`SettleOutcome::TaskLimit`]. A frame already started finishes first.
//!
//! # External waits: what the host must do
//!
//! `WaitingExternal` is not an end. Tasks already due run first, up to the
//! task limit, and then the call returns, but the page has not finished:
//! with the clock held, no later timer has run, so printing it now would
//! print a page that never got to run them. The virtual budget and the task
//! limit can't end a wait, because neither moves while the clock is held.
//! Calling again without ending a wait gives the same report for ever. So
//! the host, and only the host, ends every wait:
//!
//! 1. [`EventLoopHandle::external_begin`] returns an [`ExternalWait`] for
//!    each thing the page waits on. [`EventLoopHandle::external_end`] closes
//!    that one and says whether it was still open.
//! 2. To give up, [`EventLoopHandle::external_abandon_all`] closes every
//!    open wait. A result that arrives later finds `external_end` returning
//!    false: drop it. It can't close a newer wait, even one in the next
//!    render of a reset or pooled sandbox: an [`ExternalWait`] is unique in
//!    the process.
//! 3. Call [`Sandbox::run_until_settled`](crate::Sandbox) again. The timers
//!    then run to the budget. The time a wait held is not taken from the
//!    budget, since the clock didn't move.
//!
//! When the host gives up doesn't change the output: the clock didn't move
//! and no task ran meanwhile, so giving up after no calls or after many
//! gives the same report and the same page. Which waits it gave up on can:
//! that is the one way real time can reach a page, and only through the
//! host's choice.
//!
//! A page can open waits in a chain (an image's `onerror` setting another
//! `src`), each closing before the next, and the clock stays held all the
//! way. So the loader (E2-06) caps its waits for each render, however they
//! end: 1,000 started, and 10 seconds of real time from the first. At either
//! cap it calls `external_abandon_all`, fails the rest of its resources
//! without opening a wait, and lets the page settle.
//!
//! The virtual budget and the task count are measured from the start of the
//! context, not of the call, so calling again can't extend them. The real
//! time, memory and stack limits apply to the whole of a
//! `run_until_settled` call, which is one call to the sandbox: every
//! callback, microtask and check between them runs under the same deadline.
//! A callback that throws is reported ([`SettleReport::errors`]) and the
//! loop goes on, as in a browser; hitting a limit ends the call with the
//! typed error.
//!
//! # Rejected promises
//!
//! An exception inside an async function or a promise reaction doesn't
//! escape: it rejects a promise, and if no script handles that promise,
//! nothing else would ever see it. That includes a stack overflow, the one
//! limit that isn't sticky. So the runtime's promise rejection tracker
//! ([`rejection_tracker`]) does what HTML's "unhandled promise rejections"
//! section says: a promise rejected without a handler goes on a list, a
//! handler added to it takes it off, and whatever is left when the microtasks
//! have drained was not handled. For each of those, in the order they
//! rejected:
//!
//! - if the reason is the engine's own stack-overflow or out-of-memory error,
//!   the call ends with that typed error, as if it had been thrown from a
//!   timer. The engine marks those errors itself, so a script can't forge one
//!   or disguise one. Handling the promise is like catching the error, and
//!   stops it the same way;
//! - any other reason is counted ([`SettleReport::unhandled_rejections`]),
//!   and the loop goes on. The first few are read and kept
//!   ([`SettleReport::rejections`]); past those, nothing reads a reason's
//!   properties, so none of its getters run.
//!
//! That is the whole story for a sandbox with no hook. With one
//! ([`LoopBridge::set_rejection_hook`], how the DOM fires
//! `unhandledrejection`), the second case follows "notify about rejected
//! promises" to the letter: the checkpoint queues a task, an ordinary one
//! behind whatever is already queued for this time, and the promises it took
//! stay on the list until that task runs, so a handler added before then (by
//! the next `<script>`, or a timer already due) still takes one off. The task
//! calls the hook with `(promise, reason)` for each promise left, and counts
//! it only if the hook returns true. A limit error doesn't wait for the task
//! and no script ever sees one: it ends the call at the checkpoint, as above.
//!
//! Each checkpoint that took promises queues its own task, for those promises
//! only. These tasks count towards [`SettleOptions::max_tasks`]. If the call
//! stops with one still queued, the promises it holds are counted with no
//! call to the hook, so the report loses none; `max_tasks: 0` asks for a
//! checkpoint and no more, so it leaves them for a later call.
//!
//! The checkpoint is the event loop's alone: a call such as
//! [`Sandbox::eval`](crate::Sandbox) runs microtasks but reports no
//! rejection, so it counts as one task with the next
//! [`Sandbox::run_until_settled`](crate::Sandbox), up to that call's first
//! drain. A handler added once a promise has been counted changes nothing,
//! and `rejectionhandled` is not tracked.
//!
//! The list holds the promise and its reason as persistent references, the
//! only JavaScript values Rust holds besides the dispatcher and the hook, and
//! only until its task. The sandbox empties it before the runtime goes. Its
//! entries are outside the JavaScript heap, so they are capped like timers
//! ([`MAX_PENDING_REJECTIONS`]).
//!
//! # A checkpoint from a native
//!
//! HTML's "clean up after running script" performs a microtask checkpoint
//! each time the JavaScript stack empties, and that is what puts one between
//! the listeners of an event the browser fires. The DOM runs those listeners
//! from one piece of glue, so it asks for that checkpoint with
//! [`LoopBridge::checkpoint`]. Only the caller knows that no script of the
//! page is on the stack: a checkpoint under a running script would run
//! microtasks in the middle of it.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rquickjs::runtime::RejectionTracker;
use rquickjs::{Ctx, Exception, Function, JsLifetime, Object, Persistent, Value};

use crate::SandboxError;
use crate::engine::{self, LimitState};
use crate::sandbox::{classify, drain_jobs, eval_options, exception_error, guard_native};

/// Most timers alive at once. Their Rust entries aren't in the JavaScript
/// heap, so this is what bounds them; far above what real pages use.
pub(crate) const MAX_LIVE_TIMERS: usize = 100_000;

/// Most rejected promises waiting for a handler at once. Their Rust entries
/// aren't in the JavaScript heap, so this is what bounds them; passing it
/// fails the call with [`SandboxError::MemoryLimit`].
pub(crate) const MAX_PENDING_REJECTIONS: usize = 100_000;

/// Most uncaught exceptions kept for [`SettleReport::errors`], and most
/// unhandled rejections for [`SettleReport::rejections`]; the rest are only
/// counted.
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
    /// Most tasks (timer and animation-frame callbacks, and the tasks that
    /// announce rejected promises) to run in the life of the context. The
    /// limit is checked between frames, so the rest of a frame already
    /// started can pass it. Microtasks don't count; the time limit stops
    /// those.
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
/// the microtasks are drained and script state is consistent. `Settled` and
/// `TaskLimit` are ends, after which the page can be serialized;
/// `WaitingExternal` is not (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleOutcome {
    /// Nothing is left to do within the budget.
    Settled,
    /// Tasks were still due when the task limit was reached.
    TaskLimit,
    /// External waits are pending and the clock can't move until they end.
    /// Not an end: neither limit can stop a wait, so the host must end every
    /// wait or abandon them, then call again, and calling again without
    /// doing so returns this for ever. Timers may be waiting too
    /// ([`SettleReport::timers_left`]).
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
    /// Timers and animation-frame callbacks that haven't run yet.
    pub timers_left: usize,
    /// External waits still pending.
    pub external_pending: usize,
    /// Exceptions no script caught since the last report, in callbacks of
    /// timers, frames and `queueMicrotask`.
    pub uncaught: u64,
    /// The first few of those.
    pub errors: Vec<SandboxError>,
    /// Promises rejected with no handler at the end of a microtask
    /// checkpoint since the last report, the ones `unhandledrejection` is
    /// fired for. Never a limit error: that ends the call.
    pub unhandled_rejections: u64,
    /// The first few of those reasons, in the order they rejected.
    pub rejections: Vec<SandboxError>,
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
/// reset or a limit error (an old one counts on a loop that is gone). An
/// [`ExternalWait`] from before that closes nothing on the new one.
#[derive(Debug, Clone)]
pub struct EventLoopHandle {
    pub(crate) shared: Shared,
}

impl EventLoopHandle {
    /// Records that the page now waits on one more external thing.
    pub fn external_begin(&self) -> ExternalWait {
        // Never reaches the page, so sharing it across sandboxes can't make
        // one render depend on another.
        let id = NEXT_WAIT.fetch_add(1, Ordering::Relaxed);
        self.shared.borrow_mut().external.insert(id);
        ExternalWait(id)
    }

    /// Ends `wait`. True if it was open; false if it had ended or been
    /// abandoned, when the host must drop whatever result it has for it.
    /// Never touches any other wait.
    pub fn external_end(&self, wait: ExternalWait) -> bool {
        self.shared.borrow_mut().external.remove(&wait.0)
    }

    /// Gives up on every open wait, returning how many there were. Waits
    /// begun afterwards are new ones.
    pub fn external_abandon_all(&self) -> usize {
        let mut st = self.shared.borrow_mut();
        let open = st.external.len();
        st.external.clear();
        open
    }

    /// External waits pending now.
    pub fn external_pending(&self) -> usize {
        self.shared.borrow().external.len()
    }

    /// The virtual clock, in milliseconds since the start of the context.
    pub fn virtual_now_ms(&self) -> u64 {
        self.shared.borrow().now
    }
}

/// The id of the next [`ExternalWait`], over the whole process: a token
/// outlives the runtime it came from, and may cross to another thread.
static NEXT_WAIT: AtomicU64 = AtomicU64::new(0);

/// One external wait, from [`EventLoopHandle::external_begin`]. Ids are never
/// reused in the process, by any sandbox or after any reset, so a late end
/// can only find its own wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalWait(u64);

/// What a timer does when it comes due. The numbers are the `kind` argument
/// of the `schedule` native (see `prelude.js`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Timeout = 0,
    Interval = 1,
    Frame = 2,
    /// Not a timer and never from a script: the task "notify about rejected
    /// promises" queues. It has no live id.
    Rejections = 3,
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

/// [`Key`] class of an animation frame's callbacks.
const CLASS_FRAME: u8 = 0;
/// [`Key`] class of timers.
const CLASS_TIMER: u8 = 1;

/// Queue order: due time; frames before timers, so no timer due on a frame's
/// tick can land inside the frame (HTML runs a frame's callbacks in one step
/// of "update the rendering"); then the order they were set (the standard's
/// "run steps after a timeout" gives the same order for timers of one
/// ordering identifier).
type Key = (u64, u8, u64);

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
    /// For [`Kind::Rejections`]: it announces the promises with a sequence
    /// number below this.
    rejections_before: u64,
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
    /// except from the front, so order is always the [`Key`]'s.
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
    /// Due time of the last frame started: more frame callbacks at this time
    /// are the rest of that frame.
    open_frame: Option<u64>,
    tasks_run: u64,
    /// Open external waits.
    external: BTreeSet<u64>,
    uncaught: u64,
    errors: Vec<SandboxError>,
    unhandled_rejections: u64,
    rejections: Vec<SandboxError>,
    /// A limit error found in a callback's report: the call must end.
    fatal: Option<SandboxError>,
    /// Tasks of [`Kind::Rejections`] in the queue.
    rejection_tasks: usize,
    /// A native's checkpoint is running: HTML's "performing a microtask
    /// checkpoint" flag, for the only checkpoints that can nest.
    in_native_checkpoint: bool,
}

pub(crate) type Shared = Rc<RefCell<LoopState>>;

/// One rejected promise waiting for a handler.
struct PendingRejection {
    /// Holding it also keeps its address its own.
    promise: Persistent<Value<'static>>,
    reason: Persistent<Value<'static>>,
    address: usize,
    /// The limit error the reason is, if it is one.
    limit: Option<SandboxError>,
}

/// HTML's "about-to-be-notified rejected promises list", and the promises
/// queued tasks are still to look at.
#[derive(Default)]
pub(crate) struct Rejections {
    /// By the order they rejected: addresses can't decide anything a page
    /// sees, or two runs would differ.
    pending: BTreeMap<u64, PendingRejection>,
    /// Where each promise is in `pending`, because handlers can come in any
    /// order and a script can have many rejections waiting.
    by_address: HashMap<usize, u64>,
    next_seq: u64,
    /// Entries below this have a task queued; the rest wait for a checkpoint.
    queued_before: u64,
    /// Called for each promise a task finds still unhandled; see the module
    /// docs. Returns whether to count it.
    hook: Option<Persistent<Function<'static>>>,
    /// The runtime is going: nothing may take a reference from here on.
    closed: bool,
}

pub(crate) type SharedRejections = Rc<RefCell<Rejections>>;

impl Rejections {
    fn add(&mut self, entry: PendingRejection) {
        self.by_address.insert(entry.address, self.next_seq);
        self.pending.insert(self.next_seq, entry);
        self.next_seq += 1;
    }

    /// Takes a promise that just got a handler off the list. One that isn't
    /// there has been reported already, and stays reported.
    fn handled(&mut self, address: usize) -> Option<PendingRejection> {
        let seq = self.by_address.remove(&address)?;
        self.pending.remove(&seq)
    }

    /// The first entry below sequence number `end`.
    fn pop_before(&mut self, end: u64) -> Option<PendingRejection> {
        let entry = self.pending.first_entry().filter(|e| *e.key() < end)?;
        let entry = entry.remove();
        self.by_address.remove(&entry.address);
        Some(entry)
    }

    /// Whether a checkpoint has anything to look at.
    fn has_new(&self) -> bool {
        self.pending.range(self.queued_before..).next().is_some()
    }

    /// Lets go of every reference, which must happen before the runtime
    /// goes.
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.by_address.clear();
        self.hook = None;
        self.closed = true;
    }
}

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
        let (due, class) = match kind {
            // Strictly later: one set during a frame waits for the next.
            Kind::Frame => ((self.now / FRAME_MS + 1) * FRAME_MS, CLASS_FRAME),
            Kind::Timeout | Kind::Interval | Kind::Rejections => {
                let wait = if nesting > CLAMP_NESTING {
                    timeout.max(CLAMP_MS)
                } else {
                    timeout
                };
                (self.now + u64::from(wait), CLASS_TIMER)
            }
        };
        let key = (due, class, self.next_seq);
        self.next_seq += 1;
        self.queue.insert(
            key,
            Task {
                id,
                kind,
                timeout,
                nesting: nesting.saturating_add(1),
                rejections_before: 0,
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

    /// "Queue a global task" in "notify about rejected promises": a task due
    /// now, behind everything already queued for now. Each needs a checkpoint
    /// with a new rejection, so tasks and calls bound them as timers bound
    /// themselves.
    fn queue_rejections(&mut self, before: u64) {
        let key = (self.now, CLASS_TIMER, self.next_seq);
        self.next_seq += 1;
        self.rejection_tasks += 1;
        self.queue.insert(
            key,
            Task {
                id: 0,
                kind: Kind::Rejections,
                timeout: 0,
                nesting: 0,
                rejections_before: before,
            },
        );
    }

    /// Drops the queued [`Kind::Rejections`] tasks.
    fn drop_rejection_tasks(&mut self) {
        self.queue.retain(|_, task| task.kind != Kind::Rejections);
        self.rejection_tasks = 0;
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
        let budget = options.virtual_budget_ms;
        let waiting = !self.external.is_empty();
        let now = self.now;
        let Some(entry) = self
            .queue
            .first_entry()
            .filter(|e| e.key().0 <= budget && !(waiting && e.key().0 > now))
        else {
            return Step::Stop(if waiting {
                SettleOutcome::WaitingExternal
            } else {
                SettleOutcome::Settled
            });
        };
        let (due, class, _) = *entry.key();
        let mid_frame = class == CLASS_FRAME && self.open_frame == Some(due);
        if self.tasks_run >= options.max_tasks && !mid_frame {
            return Step::Stop(SettleOutcome::TaskLimit);
        }
        let task = entry.remove();
        self.now = due;
        self.tasks_run += 1;
        if let Some(slot) = self.live.get_mut(&task.id) {
            slot.key = None;
        }
        self.nesting = match task.kind {
            Kind::Frame => {
                self.open_frame = Some(due);
                0
            }
            Kind::Timeout | Kind::Interval => task.nesting,
            Kind::Rejections => {
                self.rejection_tasks -= 1;
                0
            }
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

    /// Records a promise no script handled.
    fn note_rejection(&mut self, reason: SandboxError) {
        self.unhandled_rejections += 1;
        if self.rejections.len() < MAX_KEPT_ERRORS {
            self.rejections.push(reason);
        }
    }

    fn report(&mut self, outcome: SettleOutcome) -> SettleReport {
        let errors = std::mem::take(&mut self.errors);
        SettleReport {
            outcome,
            virtual_now_ms: self.now,
            tasks_run: self.tasks_run,
            timers_left: self.queue.len(),
            external_pending: self.external.len(),
            uncaught: std::mem::take(&mut self.uncaught),
            errors,
            unhandled_rejections: std::mem::take(&mut self.unhandled_rejections),
            rejections: std::mem::take(&mut self.rejections),
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
    let stop = limit.clone();
    host.set(
        "runScript",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, source: String| -> rquickjs::Result<()> {
                guard_native(&stop, &ctx, || {
                    let options = eval_options("<timer>");
                    match ctx.eval_with_options::<Value<'js>, _>(source, options) {
                        Ok(_) => Ok(()),
                        Err(rquickjs::Error::Exception) => Err(rquickjs::Error::Exception),
                        // rquickjs takes a C string, so a NUL can't reach the
                        // parser, which is all that ends up here.
                        Err(_) => Err(Exception::throw_syntax(&ctx, "invalid character in script")),
                    }
                })
            },
        )?,
    )?;
    Ok(host)
}

/// The runtime's promise rejection tracker: HTML's
/// HostPromiseRejectionTracker. The engine calls it in the middle of rejecting
/// or subscribing to a promise, so it runs no script and allocates nothing in
/// the JavaScript heap.
pub(crate) fn rejection_tracker(
    rejections: &SharedRejections,
    limit: &Arc<LimitState>,
) -> RejectionTracker {
    let (rejections, stop) = (rejections.clone(), limit.clone());
    Box::new(move |ctx, promise, reason, is_handled| {
        let address = engine::object_address(&promise);
        if is_handled {
            // Bound first: dropping an entry frees JavaScript references, so
            // it happens after the borrow.
            let entry = rejections.borrow_mut().handled(address);
            drop(entry);
            return;
        }
        let mut list = rejections.borrow_mut();
        if list.pending.len() >= MAX_PENDING_REJECTIONS {
            stop.exceed_memory();
            return;
        }
        list.add(PendingRejection {
            limit: engine::limit_error(&reason),
            promise: Persistent::save(&ctx, promise),
            reason: Persistent::save(&ctx, reason),
            address,
        });
    })
}

/// HTML's "notify about rejected promises", at the end of a microtask
/// checkpoint. Without a hook it decides here, with [`count_now`]; with one,
/// [`announce`] does, in the task queued here. Either way a limit error ends
/// the call now, and through no script.
fn notify_rejections<'js>(
    ctx: &Ctx<'js>,
    limit: &LimitState,
    shared: &Shared,
    rejections: &SharedRejections,
) -> Result<(), SandboxError> {
    let mut list = rejections.borrow_mut();
    if list.hook.is_none() {
        drop(list);
        return count_now(ctx, limit, shared, rejections, u64::MAX);
    }
    let new = list.pending.range(list.queued_before..);
    if let Some(error) = new.filter_map(|(_, entry)| entry.limit.clone()).next() {
        return Err(error);
    }
    list.queued_before = list.next_seq;
    shared.borrow_mut().queue_rejections(list.next_seq);
    Ok(())
}

/// Counts the entries below `end`, in order, with no hook. Reading a hostile
/// reason can run its getters, so no entry is taken before the last one is
/// done with: a getter may still handle one.
///
/// Returns `Ok` with entries left if a limit stops it: the caller's next
/// [`drain_jobs`] gives the typed error, and the rebuild that follows drops
/// the rest.
fn count_now<'js>(
    ctx: &Ctx<'js>,
    limit: &LimitState,
    shared: &Shared,
    rejections: &SharedRejections,
    end: u64,
) -> Result<(), SandboxError> {
    loop {
        // Checked before every entry, like every job in `drain_jobs`: a reason
        // with plain data properties runs no script, so no interrupt poll
        // would stop this loop, and copying its strings is not free.
        if limit.should_interrupt() {
            return Ok(());
        }
        // Bound first, so the borrow ends before any script can run.
        let next = rejections.borrow_mut().pop_before(end);
        let Some(entry) = next else {
            return Ok(());
        };
        count_rejection(ctx, shared, restore(ctx, entry.reason)?)?;
    }
}

/// The task "notify about rejected promises" queues: the hook gets each
/// promise below `end` that no handler took off the list by now, and a true
/// answer counts it. Bound by the same checks as [`count_now`].
fn announce<'js>(
    ctx: &Ctx<'js>,
    limit: &LimitState,
    shared: &Shared,
    rejections: &SharedRejections,
    end: u64,
) -> Result<(), SandboxError> {
    let hook = rejections.borrow().hook.clone();
    let Some(hook) = hook else {
        return count_now(ctx, limit, shared, rejections, end);
    };
    let hook = restore(ctx, hook)?;
    loop {
        if limit.should_interrupt() {
            return Ok(());
        }
        let next = rejections.borrow_mut().pop_before(end);
        let Some(entry) = next else {
            return Ok(());
        };
        let reason = restore(ctx, entry.reason)?;
        let args = (restore(ctx, entry.promise)?, reason.clone());
        let count = match hook.call::<_, bool>(args) {
            Ok(count) => count,
            Err(e) => {
                survive(ctx, limit, shared, e)?;
                true
            }
        };
        if count {
            count_rejection(ctx, shared, reason)?;
        }
    }
}

/// A persistent reference back as a value.
fn restore<'js, T>(ctx: &Ctx<'js>, saved: Persistent<T>) -> Result<T::Changed<'js>, SandboxError>
where
    T: JsLifetime<'static>,
{
    saved.restore(ctx).map_err(|e| SandboxError::Engine {
        message: e.to_string(),
    })
}

/// Records one promise nothing handled. A reason nobody keeps isn't read:
/// that bounds the copying per checkpoint, and no getter runs for nothing.
/// The mark needs no read.
fn count_rejection<'js>(
    ctx: &Ctx<'js>,
    shared: &Shared,
    reason: Value<'js>,
) -> Result<(), SandboxError> {
    if shared.borrow().rejections.len() >= MAX_KEPT_ERRORS {
        if let Some(error) = engine::limit_error(&reason) {
            return Err(error);
        }
        shared.borrow_mut().unhandled_rejections += 1;
        return Ok(());
    }
    let error = exception_error(reason);
    let _ = ctx.catch();
    if error.is_limit() {
        return Err(error);
    }
    shared.borrow_mut().note_rejection(error);
    Ok(())
}

/// A microtask checkpoint: drains the microtasks, then reports the promises
/// left rejected with no handler. An exception escaping a job is reported and
/// the rest still run; a limit ends the call.
fn drain<'js>(
    ctx: &Ctx<'js>,
    limit: &LimitState,
    shared: &Shared,
    rejections: &SharedRejections,
) -> Result<(), SandboxError> {
    shared.borrow_mut().nesting = 0;
    loop {
        while let Err(e) = drain_jobs(ctx, limit) {
            survive(ctx, limit, shared, e)?;
        }
        if let Some(fatal) = shared.borrow_mut().fatal.take() {
            return Err(fatal);
        }
        if !rejections.borrow().has_new() {
            return Ok(());
        }
        notify_rejections(ctx, limit, shared, rejections)?;
        // A reason's getters may have queued more of either, and a limit may
        // have stopped it early: `drain_jobs` reports that.
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

/// Not public API: what `surimono-dom` needs from the event loop of the
/// context it installs into, from [`Sandbox::loop_bridge`](crate::Sandbox).
/// Good for that context only: past its end both methods do nothing, so take
/// a new one for each context.
#[doc(hidden)]
pub struct LoopBridge {
    pub(crate) shared: Shared,
    pub(crate) rejections: SharedRejections,
    pub(crate) limit: Arc<LimitState>,
}

impl std::fmt::Debug for LoopBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopBridge").finish_non_exhaustive()
    }
}

impl LoopBridge {
    /// Registers `hook` as the function a task calls with `(promise, reason)`
    /// for each promise left rejected with no handler; see the module docs.
    /// It returns whether to count the promise. It is called with no script
    /// on the stack, and never with a limit error.
    pub fn set_rejection_hook<'js>(&self, ctx: &Ctx<'js>, hook: Function<'js>) {
        let mut list = self.rejections.borrow_mut();
        if !list.closed {
            list.hook = Some(Persistent::save(ctx, hook));
        }
    }

    /// A microtask checkpoint from inside a native: the microtasks, then the
    /// rejected promises, as between two tasks. For a native called with no
    /// script of the page on the stack; see the module docs. Does nothing
    /// inside another such checkpoint.
    ///
    /// An exception escaping a microtask is reported as usual. An error here
    /// is the end of the call (a limit, or a cancel): the native must stop
    /// the script, and the call must end with that error.
    pub fn checkpoint<'js>(&self, ctx: &Ctx<'js>) -> Result<(), SandboxError> {
        if self.rejections.borrow().closed {
            return Ok(());
        }
        if std::mem::replace(&mut self.shared.borrow_mut().in_native_checkpoint, true) {
            return Ok(());
        }
        let result = drain(ctx, &self.limit, &self.shared, &self.rejections);
        self.shared.borrow_mut().in_native_checkpoint = false;
        result
    }
}

/// The loop itself; see the module docs. `fire` is the prelude's dispatcher.
pub(crate) fn run<'js>(
    ctx: &Ctx<'js>,
    limit: &LimitState,
    shared: &Shared,
    rejections: &SharedRejections,
    fire: Function<'js>,
    options: &SettleOptions,
) -> Result<SettleReport, SandboxError> {
    loop {
        drain(ctx, limit, shared, rejections)?;
        let step = shared.borrow_mut().step(options);
        let (task, due) = match step {
            Step::Run(task, due) => (task, due),
            Step::Stop(outcome) => {
                // `max_tasks: 0` is a checkpoint, with the tasks still to run.
                if options.max_tasks == 0 || shared.borrow().rejection_tasks == 0 {
                    return Ok(shared.borrow_mut().report(outcome));
                }
                shared.borrow_mut().drop_rejection_tasks();
                let end = rejections.borrow().queued_before;
                count_now(ctx, limit, shared, rejections, end)?;
                // A reason's getters may have left work, as in `drain`.
                continue;
            }
        };
        // The borrow has ended: the callback calls the natives. The clock
        // fits a JavaScript number: `validate` bounds the budget.
        if task.kind == Kind::Rejections {
            announce(ctx, limit, shared, rejections, task.rejections_before)?;
        } else {
            let frame = task.kind == Kind::Frame;
            if let Err(e) = fire.call::<_, ()>((task.id, due as f64, frame)) {
                survive(ctx, limit, shared, e)?;
            }
        }
        drain(ctx, limit, shared, rejections)?;
        shared.borrow_mut().finish(task);
    }
}

#[cfg(test)]
mod tests {
    use rquickjs::{Context, Runtime};

    use super::*;

    /// Deterministic where a timed test can't be: past the limit, not one more
    /// entry is read, and the entries stay for the rebuild to drop.
    #[test]
    fn a_limit_stops_the_notifications_between_entries() {
        let rt = Runtime::new().unwrap();
        let ctx = Context::full(&rt).unwrap();
        let limit = LimitState::new(usize::MAX);
        let shared = Shared::default();
        let rejections = SharedRejections::default();
        rt.set_host_promise_rejection_tracker(Some(rejection_tracker(&rejections, &limit)));
        ctx.with(|ctx| {
            ctx.eval::<(), _>("Promise.reject(1); Promise.reject(2); Promise.reject(3)")
                .unwrap();
            assert_eq!(rejections.borrow().pending.len(), 3);
            limit.exceed_memory();
            assert_eq!(
                notify_rejections(&ctx, &limit, &shared, &rejections),
                Ok(())
            );
            assert_eq!(shared.borrow().unhandled_rejections, 0);
            assert_eq!(rejections.borrow().pending.len(), 3);
            // `drain` then ends the call instead of going round again.
            let drained = drain(&ctx, &limit, &shared, &rejections);
            assert_eq!(drained, Err(SandboxError::MemoryLimit));
        });
        rejections.borrow_mut().clear();
    }

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
