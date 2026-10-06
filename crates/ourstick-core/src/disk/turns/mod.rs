//! OurStick's reading from one disk, one window at a time (ADR 0013, rule
//! 5): readers (a stick's catalogue, the files it makes ready) take turns in
//! the order they asked, a file that readers wait for first but at most three to
//! one; each window is timed (`Timed`) and judged by the disk's
//! [`DiskShare`], and its pause is the disk's, so the next turn starts
//! after it whoever takes it.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::disk::share::{DiskLoad, DiskShare};

mod id;
mod shared;
mod timed;

pub use id::DiskId;
pub use shared::SharedReader;
use timed::Window;
pub use timed::{DiskReader, Readers, Timed};

/// A clock that never goes back, and a way to wait (blocking): real in the
/// app and the CLI, a fake in tests. Not the core's `Clock`, which tells
/// wall time in milliseconds for records and can jump.
pub trait Timer: Send + Sync {
    /// Time since some fixed start.
    fn elapsed(&self) -> Duration;
    fn sleep(&self, duration: Duration);
}

/// The system's monotonic clock.
#[derive(Debug)]
pub struct SystemTimer(Instant);

impl Default for SystemTimer {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl Timer for SystemTimer {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// A pause is slept in slices this long at most, checking between them
/// whether to stop (a stick stopped, its drive ejected).
const PAUSE_SLICE: Duration = Duration::from_secs(5);
/// A file that readers wait for goes first, but at most this many
/// windows in a row while a catalogue waits, so new files still come.
const URGENT_IN_A_ROW: u32 = 3;
/// A reader waiting in the queue asks this often (real time) whether to go
/// on, so a stopped stick leaves the queue at once, not at its turn.
const QUEUE_CHECK: Duration = Duration::from_millis(500);
/// A turn whose holder has read nothing for this long (a transfers' reader
/// waiting on the network) is taken by the next reader in the queue.
const IDLE_GRACE: Duration = Duration::from_millis(50);

/// Every disk OurStick reads from in this process, each with its turns.
/// Made once by the app or the CLI and given to every stick.
pub struct DiskTurns {
    timer: Arc<dyn Timer>,
    disks: Mutex<HashMap<DiskId, Arc<Disk>>>,
}

impl DiskTurns {
    pub fn new(timer: Arc<dyn Timer>) -> Self {
        Self {
            timer,
            disks: Mutex::new(HashMap::new()),
        }
    }

    /// The disk `folder` is on. Blocking.
    pub fn of(&self, folder: &Path) -> Arc<Disk> {
        self.get(DiskId::of(folder))
    }

    pub fn get(&self, id: DiskId) -> Arc<Disk> {
        lock(&self.disks)
            .entry(id)
            .or_insert_with(|| Arc::new(Disk::new(self.timer.clone())))
            .clone()
    }
}

impl std::fmt::Debug for DiskTurns {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("DiskTurns").finish_non_exhaustive()
    }
}

/// One disk: its judgement and its queue of turns.
pub struct Disk {
    timer: Arc<dyn Timer>,
    state: Mutex<DiskState>,
    turn_free: Condvar,
}

struct DiskState {
    load: Box<dyn DiskLoad>,
    /// Waiting for a turn, in the order they asked: (ticket, urgent).
    waiting: VecDeque<(u64, bool)>,
    next_ticket: u64,
    held: bool,
    /// Whether the turn held is an urgent reader's.
    held_urgent: bool,
    /// Turns granted so far: the current one's id.
    turn: u64,
    /// Since when (timer time) the holder reads nothing, if it said so:
    /// after [`IDLE_GRACE`] the next reader may take the turn.
    idle_since: Option<Duration>,
    /// The disk's pause: no turn starts before this (timer time).
    not_before: Duration,
    urgent_in_a_row: u32,
    /// Windows judged, for tests.
    windows: u64,
}

impl Default for DiskState {
    fn default() -> Self {
        Self {
            load: Box::new(DiskShare::default()),
            waiting: VecDeque::new(),
            next_ticket: 0,
            held: false,
            held_urgent: false,
            turn: 0,
            idle_since: None,
            not_before: Duration::ZERO,
            urgent_in_a_row: 0,
            windows: 0,
        }
    }
}

impl Disk {
    fn new(timer: Arc<dyn Timer>) -> Self {
        Self {
            timer,
            state: Mutex::new(DiskState::default()),
            turn_free: Condvar::new(),
        }
    }

    /// Whether OurStick gives way on this disk now. (A disk's own, not
    /// [`Readers::giving_way`]'s, which asks every disk a reader used.)
    pub fn giving_way(&self) -> bool {
        lock(&self.state).load.giving_way()
    }

    /// Waits for this reader's turn, then for the disk's pause, asking
    /// `go_on` meanwhile: an error leaves the queue (or gives the turn
    /// back) at once. Blocking.
    fn take_turn(&self, urgent: bool, go_on: &dyn Fn() -> io::Result<()>) -> io::Result<u64> {
        let mut state = lock(&self.state);
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        state.waiting.push_back((ticket, urgent));
        loop {
            if state.may_start(ticket, self.timer.elapsed()) {
                break;
            }
            // Asked without the disk's lock: it may look at a drive.
            drop(state);
            let asked = go_on();
            state = lock(&self.state);
            if let Err(error) = asked {
                state.waiting.retain(|(waiting, _)| *waiting != ticket);
                drop(state);
                // Its leaving may make another the chosen one.
                self.turn_free.notify_all();
                return Err(error);
            }
            // A turn given back while it was asked is not waited for.
            if state.may_start(ticket, self.timer.elapsed()) {
                break;
            }
            // An idle holder's turn may be taken soon: look again then.
            let wait = if state.idle_since.is_some() {
                IDLE_GRACE
            } else {
                QUEUE_CHECK
            };
            state = self
                .turn_free
                .wait_timeout(state, wait)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        state.grant(ticket);
        let turn = state.turn;
        let not_before = state.not_before;
        drop(state);
        // The disk's pause, slept holding the turn: no one would read
        // meanwhile anyway, and the turn's order is kept.
        loop {
            if let Err(error) = go_on() {
                self.give_back(None);
                return Err(error);
            }
            let now = self.timer.elapsed();
            if now >= not_before {
                return Ok(turn);
            }
            self.timer.sleep((not_before - now).min(PAUSE_SLICE));
        }
    }

    /// Whether turn `turn` is still held (an idle one may have been taken
    /// by the next reader); if so, it is no longer idle.
    fn resume(&self, turn: u64) -> bool {
        let mut state = lock(&self.state);
        let holds = state.held && state.turn == turn;
        if holds {
            state.idle_since = None;
        }
        holds
    }

    /// Turn `turn`'s holder reads nothing for now: after [`IDLE_GRACE`] the
    /// next reader may take it.
    fn go_idle(&self, turn: u64) {
        let mut state = lock(&self.state);
        if state.held && state.turn == turn {
            state.idle_since = Some(self.timer.elapsed());
            drop(state);
            self.turn_free.notify_all();
        }
    }

    /// Ends turn `turn` as [`Disk::give_back`] does, if it is still held.
    fn give_back_turn(&self, turn: u64, window: Option<&Window>) {
        if self.resume(turn) {
            self.give_back(window);
        } else if let Some(window) = window.filter(|window| window.bytes > 0) {
            // Taken while idle: the window read is still judged.
            let mut state = lock(&self.state);
            state.windows += 1;
            let pause = state.load.after_window(
                window.kind,
                window.timed_files,
                window.timed_bytes,
                window.timed,
            );
            let not_before = self.timer.elapsed() + pause.max(state.load.pause_for(window.took));
            state.not_before = state.not_before.max(not_before);
        }
    }

    /// Ends a turn; with a window read in it, judges it and sets the
    /// disk's pause.
    fn give_back(&self, window: Option<&Window>) {
        let mut state = lock(&self.state);
        // A turn that read nothing (only a file's end) leaves the pause, and
        // the judgement, as they were.
        if let Some(window) = window.filter(|window| window.bytes > 0) {
            state.window_read();
            let pause = if window.timed_files > 0 || window.timed_bytes > 0 {
                state.windows += 1;
                state.load.after_window(
                    window.kind,
                    window.timed_files,
                    window.timed_bytes,
                    window.timed,
                )
            } else {
                Duration::ZERO
            };
            // The untimed part (a second read) paused for as well.
            let pause = pause.max(state.load.pause_for(window.took));
            state.not_before = self.timer.elapsed() + pause;
        }
        state.held = false;
        drop(state);
        self.turn_free.notify_all();
    }
}

impl DiskState {
    /// Whose turn is next: the first urgent one, unless urgent ones read
    /// [`URGENT_IN_A_ROW`] windows in a row while others wait; else the
    /// first.
    fn chosen(&self) -> Option<u64> {
        let others_wait = self.waiting.iter().any(|(_, urgent)| !urgent);
        let urgent = self.waiting.iter().find(|(_, urgent)| *urgent);
        match urgent {
            Some((ticket, _)) if !others_wait || self.urgent_in_a_row < URGENT_IN_A_ROW => {
                Some(*ticket)
            }
            _ => self.waiting.front().map(|(ticket, _)| *ticket),
        }
    }

    fn grant(&mut self, ticket: u64) {
        let index = self
            .waiting
            .iter()
            .position(|(waiting, _)| *waiting == ticket)
            .expect("a granted ticket is waiting");
        let (_, urgent) = self.waiting.remove(index).expect("found above");
        self.held = true;
        self.held_urgent = urgent;
        self.turn += 1;
        self.idle_since = None;
    }

    /// Whether `ticket` may start now: the turn is free, or held by a
    /// reader idle for [`IDLE_GRACE`] (its turn ends, unjudged), and it is
    /// `ticket`'s.
    fn may_start(&mut self, ticket: u64, now: Duration) -> bool {
        if self.chosen() != Some(ticket) {
            return false;
        }
        if self.held {
            match self.idle_since {
                Some(since) if now.saturating_sub(since) >= IDLE_GRACE => {
                    self.held = false;
                    self.idle_since = None;
                }
                _ => return false,
            }
        }
        true
    }

    /// A window judged: counted in [`URGENT_IN_A_ROW`] if an urgent reader
    /// read it while others wait. Counted by windows, not turns: a reader
    /// may read one window in several turns (transfers).
    fn window_read(&mut self) {
        let others_wait = self.waiting.iter().any(|(_, urgent)| !urgent);
        self.urgent_in_a_row = if self.held_urgent && others_wait {
            self.urgent_in_a_row + 1
        } else {
            0
        };
    }
}

/// A flag set when this is dropped: a task whose future is dropped (its
/// stick stopped) stops waiting for a disk, through its `go_on`.
#[derive(Debug, Default)]
pub struct StopOnDrop(Arc<std::sync::atomic::AtomicBool>);

impl StopOnDrop {
    /// Whether the task was dropped, for its `go_on`.
    pub fn flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.0.clone()
    }
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod queue_tests;
#[cfg(test)]
mod readers_tests;
#[cfg(test)]
mod tests;
