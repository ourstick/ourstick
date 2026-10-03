//! OurStick's reading from one disk, one window at a time (ADR 0013, rule
//! 5): readers (a stick's catalogue, its suitcases) take turns in the order
//! they asked, a suitcase that readers wait for first but at most three to
//! one; each window is timed (`Timed`) and judged by the disk's
//! [`DiskShare`], and its pause is the disk's, so the next turn starts
//! after it whoever takes it.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::disk_share::{DiskLoad, DiskShare};

mod timed;

use timed::Window;
pub use timed::{DiskReader, Timed};

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
/// A suitcase that readers wait for goes first, but at most this many
/// windows in a row while a catalogue waits, so new files still come.
const URGENT_IN_A_ROW: u32 = 3;
/// A reader waiting in the queue asks this often (real time) whether to go
/// on, so a stopped stick leaves the queue at once, not at its turn.
const QUEUE_CHECK: Duration = Duration::from_millis(500);

/// Which disk a folder is on: its filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiskId {
    /// The device id of a filesystem (Linux, macOS).
    Device(u64),
    /// A Windows volume, by its path's prefix (`C:`, also for `\\?\C:`).
    /// A `subst` letter is another prefix for a folder of another volume:
    /// OurStick reads sticks by their folders' own paths, not the letter.
    Volume(String),
    /// Not known: every such folder shares one.
    Unknown,
}

impl DiskId {
    /// The disk `folder` is on. Blocking.
    pub fn of(folder: &Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            match std::fs::metadata(folder) {
                Ok(metadata) => Self::Device(metadata.dev()),
                Err(error) => {
                    tracing::debug!(%error, "a folder's disk is not known");
                    Self::Unknown
                }
            }
        }
        #[cfg(not(unix))]
        {
            use std::path::{Component, Prefix};
            match folder.components().next() {
                Some(Component::Prefix(prefix)) => match prefix.kind() {
                    Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                        Self::Volume(format!("{}:", (letter as char).to_ascii_uppercase()))
                    }
                    _ => Self::Volume(prefix.as_os_str().to_string_lossy().to_uppercase()),
                },
                _ => Self::Unknown,
            }
        }
    }
}

/// Every disk OurStick reads from in this process, each with its turns.
/// Made once by the app or the CLI and given to every stick.
pub struct Disks {
    timer: Arc<dyn Timer>,
    disks: Mutex<HashMap<DiskId, Arc<Disk>>>,
}

impl Disks {
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

impl std::fmt::Debug for Disks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Disks").finish_non_exhaustive()
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

    /// Whether OurStick gives way on this disk now.
    pub fn giving_way(&self) -> bool {
        lock(&self.state).load.giving_way()
    }

    /// Waits for this reader's turn, then for the disk's pause, asking
    /// `go_on` meanwhile: an error leaves the queue (or gives the turn
    /// back) at once. Blocking.
    fn take_turn(&self, urgent: bool, go_on: &dyn Fn() -> io::Result<()>) -> io::Result<()> {
        let mut state = lock(&self.state);
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        state.waiting.push_back((ticket, urgent));
        loop {
            if !state.held && state.chosen() == Some(ticket) {
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
            if !state.held && state.chosen() == Some(ticket) {
                break;
            }
            state = self
                .turn_free
                .wait_timeout(state, QUEUE_CHECK)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        state.grant(ticket);
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
                return Ok(());
            }
            self.timer.sleep((not_before - now).min(PAUSE_SLICE));
        }
    }

    /// Ends a turn; with a window read in it, judges it and sets the
    /// disk's pause.
    fn give_back(&self, window: Option<&Window>) {
        let mut state = lock(&self.state);
        // A turn that read nothing (only a file's end) leaves the pause, and
        // the judgement, as they were.
        if let Some(window) = window.filter(|window| window.bytes > 0) {
            let pause = if window.timed_files > 0 || window.timed_bytes > 0 {
                state.windows += 1;
                state
                    .load
                    .after_window(window.timed_files, window.timed_bytes, window.timed)
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
    /// Whose turn is next: the first urgent one, unless urgent ones went
    /// [`URGENT_IN_A_ROW`] times while others wait; else the first.
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
        let others_wait = self.waiting.iter().any(|(_, urgent)| !urgent);
        self.urgent_in_a_row = if urgent && others_wait {
            self.urgent_in_a_row + 1
        } else {
            0
        };
        self.held = true;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
