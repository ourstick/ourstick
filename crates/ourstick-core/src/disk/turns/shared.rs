//! The transfers' reader of a disk (ADR 0017, amendment 1): every transfer
//! of a stick from one disk reads through one turn, several at once, for a
//! window (64 files or 64 MB); a turn left idle (transfers waiting on the
//! network) goes to the next reader in the queue after 50 ms.

use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use super::timed::Window;
use super::{lock, Disk, QUEUE_CHECK};
use crate::disk::share::{ReaderKind, WINDOW_BYTES, WINDOW_FILES};

/// One reader of a disk shared by threads: they read in its turn at once.
/// Its window is kept across turns and judged once full, by the wall time
/// a read was in flight (reads at once overlap). Always urgent: readers
/// wait on what it reads.
pub struct SharedReader {
    disk: Arc<Disk>,
    state: Mutex<Shared>,
    /// Woken when a turn was taken, or taking one failed.
    taken: Condvar,
}

struct Shared {
    /// The turn held, by its id.
    turn: Option<u64>,
    /// A thread is taking a turn for all.
    taking: bool,
    in_flight: u32,
    /// Since when a read is in flight (timer time).
    busy_since: Duration,
    /// The window is full: reads in flight end it, no new one joins.
    closing: bool,
    window: Window,
}

impl SharedReader {
    pub fn new(disk: Arc<Disk>) -> Self {
        Self {
            disk,
            state: Mutex::new(Shared {
                turn: None,
                taking: false,
                in_flight: 0,
                busy_since: Duration::ZERO,
                closing: false,
                window: transfers_window(),
            }),
            taken: Condvar::new(),
        }
    }

    /// Reads with `read` in this reader's turn, taking one if none is
    /// held; `read` gives what it read and how many bytes. `begins`: the
    /// first read of a file by its transfer. `go_on` is asked while
    /// waiting: an error leaves the queue at once. Blocking.
    pub fn read<T>(
        &self,
        begins: bool,
        go_on: &dyn Fn() -> io::Result<()>,
        read: impl FnOnce() -> io::Result<(T, u64)>,
    ) -> io::Result<T> {
        self.join(go_on)?;
        let mut in_flight = InFlight {
            reader: self,
            bytes: 0,
            begins,
        };
        let (value, bytes) = read()?;
        in_flight.bytes = bytes;
        Ok(value)
    }

    /// Whether OurStick gives way on this reader's disk now.
    pub fn giving_way(&self) -> bool {
        self.disk.giving_way()
    }

    /// Counts a read in, in a turn held. Asks `go_on` first.
    fn join(&self, go_on: &dyn Fn() -> io::Result<()>) -> io::Result<()> {
        go_on()?;
        let mut state = lock(&self.state);
        loop {
            if let Some(turn) = state.turn {
                if state.closing {
                    // The window is full: no new read joins it, so the reads
                    // in flight end and the turn goes back, judged.
                    state = self.wait(state, go_on)?;
                    continue;
                }
                // A read in flight keeps the turn from being idle.
                if state.in_flight > 0 || self.disk.resume(turn) {
                    if state.in_flight == 0 {
                        state.busy_since = self.disk.timer.elapsed();
                    }
                    state.in_flight += 1;
                    return Ok(());
                }
                // Taken by the next reader while idle.
                state.turn = None;
            }
            if state.taking {
                state = self.wait(state, go_on)?;
                continue;
            }
            state.taking = true;
            drop(state);
            let taken = self.disk.take_turn(true, go_on);
            state = lock(&self.state);
            state.taking = false;
            self.taken.notify_all();
            state.turn = Some(taken?);
        }
    }

    /// Waits until a turn is taken or given back, asking `go_on`.
    fn wait<'a>(
        &'a self,
        state: MutexGuard<'a, Shared>,
        go_on: &dyn Fn() -> io::Result<()>,
    ) -> io::Result<MutexGuard<'a, Shared>> {
        let state = self
            .taken
            .wait_timeout(state, QUEUE_CHECK)
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
        drop(state);
        go_on()?;
        Ok(lock(&self.state))
    }

    /// A read ended (or failed, with nothing read). A full window closes:
    /// once no read is in flight, the turn is given back and the window
    /// judged. Otherwise the turn is kept, idle: the next reader in the
    /// disk's queue takes it after [`IDLE_GRACE`](super::IDLE_GRACE).
    /// The disk is told while this reader's lock is held, so no read can
    /// join a turn told idle or given back.
    fn leave(&self, bytes: u64, begins: bool) {
        let mut state = lock(&self.state);
        state.in_flight -= 1;
        let window = &mut state.window;
        window.bytes += bytes;
        window.timed_bytes += bytes;
        if begins && bytes > 0 {
            window.timed_files += 1;
        }
        if window.timed_files >= WINDOW_FILES || window.bytes >= WINDOW_BYTES {
            state.closing = true;
        }
        if state.in_flight > 0 {
            return;
        }
        let busy = self.disk.timer.elapsed().saturating_sub(state.busy_since);
        state.window.took += busy;
        state.window.timed += busy;
        let Some(turn) = state.turn else { return };
        if state.closing {
            let window = std::mem::replace(&mut state.window, transfers_window());
            state.turn = None;
            state.closing = false;
            self.disk.give_back_turn(turn, Some(&window));
            self.taken.notify_all();
        } else {
            self.disk.go_idle(turn);
        }
    }
}

impl Drop for SharedReader {
    fn drop(&mut self) {
        let turn = lock(&self.state).turn.take();
        if let Some(turn) = turn {
            self.disk.give_back_turn(turn, None);
        }
    }
}

impl std::fmt::Debug for SharedReader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SharedReader")
            .finish_non_exhaustive()
    }
}

/// A read in flight, counted out when it ends, also by an error or a
/// panic, so the turn is never left held.
struct InFlight<'a> {
    reader: &'a SharedReader,
    bytes: u64,
    begins: bool,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.reader.leave(self.bytes, self.begins);
    }
}

fn transfers_window() -> Window {
    Window {
        kind: ReaderKind::Transfers,
        ..Window::default()
    }
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
