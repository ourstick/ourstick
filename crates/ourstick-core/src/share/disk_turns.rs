//! OurStick's reading from one disk, one window at a time (ADR 0013, rule
//! 5): readers (a stick's catalogue, its suitcases) take turns in the order
//! they asked, a suitcase that readers wait for first but at most three to
//! one; each window is timed (`Timed`) and judged by the disk's
//! [`DiskShare`], and its pause is the disk's, so the next turn starts
//! after it whoever takes it.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::disk_share::{DiskShare, WINDOW_BYTES, WINDOW_FILES};

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

/// Which disk a folder is on: its filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiskId {
    /// The device id of a filesystem (Linux, macOS).
    Device(u64),
    /// A Windows volume, by its path's prefix (`C:`).
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
                Err(_) => Self::Unknown,
            }
        }
        #[cfg(not(unix))]
        {
            match folder.components().next() {
                Some(std::path::Component::Prefix(prefix)) => {
                    Self::Volume(prefix.as_os_str().to_string_lossy().to_uppercase())
                }
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

#[derive(Default)]
struct DiskState {
    share: DiskShare,
    /// Waiting for a turn, in the order they asked: (ticket, urgent).
    waiting: VecDeque<(u64, bool)>,
    next_ticket: u64,
    held: bool,
    /// The disk's pause: no turn starts before this (timer time).
    not_before: Duration,
    urgent_in_a_row: u32,
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
        lock(&self.state).share.giving_way()
    }

    /// Waits for this reader's turn, then for the disk's pause (in slices,
    /// asking `go_on` between them). Blocking.
    fn take_turn(&self, urgent: bool, go_on: &dyn Fn() -> io::Result<()>) -> io::Result<()> {
        let mut state = lock(&self.state);
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        state.waiting.push_back((ticket, urgent));
        loop {
            if !state.held && state.chosen() == Some(ticket) {
                break;
            }
            state = self
                .turn_free
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        state.grant(ticket);
        let not_before = state.not_before;
        drop(state);
        // The pause, held by the turn: no one reads meanwhile anyway.
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
        // A turn that read nothing (only a file's end) leaves the pause as
        // it was.
        if let Some(window) = window.filter(|window| window.bytes > 0 || window.timed_files > 0) {
            let pause = if window.timed_files > 0 || window.timed_bytes > 0 {
                state
                    .share
                    .after_window(window.timed_files, window.timed_bytes, window.timed)
            } else {
                Duration::ZERO
            };
            // The untimed part (a second read) paused for as well.
            let pause = pause.max(state.share.pause_for(window.took));
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

/// What one turn has read.
#[derive(Debug, Default)]
struct Window {
    /// Files begun and their first reads' bytes and time: what is judged.
    timed_files: u64,
    timed_bytes: u64,
    timed: Duration,
    /// Every read's time, the second reads' too: what is paused for.
    took: Duration,
    bytes: u64,
}

/// One reader of a disk (a catalogue pass, a keeper's suitcase): its turn
/// and its current window. Files are read through [`DiskReader::open`].
pub struct DiskReader {
    disk: Arc<Disk>,
    urgent: bool,
    go_on: Box<dyn Fn() -> io::Result<()> + Send>,
    window: Option<Window>,
}

impl DiskReader {
    /// `urgent`: a suitcase that readers wait for. `go_on` is asked before
    /// each turn and between slices of a pause: an error (a stick stopped,
    /// its drive gone) ends the read with it.
    pub fn new(
        disk: Arc<Disk>,
        urgent: bool,
        go_on: impl Fn() -> io::Result<()> + Send + 'static,
    ) -> Self {
        Self {
            disk,
            urgent,
            go_on: Box::new(go_on),
            window: None,
        }
    }

    /// Reads `file`, of `len` bytes, through this reader's turns. Reading
    /// at its end takes no turn, so a reader does not wait for the disk's
    /// pause only to learn that a file has ended.
    pub fn open<R: Read + Seek>(&mut self, file: R, len: u64) -> Timed<'_, R> {
        Timed {
            file,
            reader: self,
            len,
            at: 0,
            begun: false,
            second: false,
        }
    }

    /// Gives the turn back now (the end of a part), judging what was read.
    pub fn end_window(&mut self) {
        if let Some(window) = self.window.take() {
            self.disk.give_back(Some(&window));
        }
    }

    /// The window to read into, taking a turn first if none is held.
    fn window(&mut self) -> io::Result<&mut Window> {
        if self.window.is_none() {
            self.disk.take_turn(self.urgent, self.go_on.as_ref())?;
            self.window = Some(Window::default());
        }
        Ok(self.window.as_mut().expect("just set"))
    }
}

impl Drop for DiskReader {
    fn drop(&mut self) {
        self.end_window();
    }
}

/// A file read through a [`DiskReader`]: its first read is timed into the
/// reader's windows, and a seek back to its start (as `encrypt_blob` reads
/// twice) makes the rest a second read, paused for but not judged.
pub struct Timed<'a, R> {
    file: R,
    reader: &'a mut DiskReader,
    len: u64,
    at: u64,
    begun: bool,
    second: bool,
}

impl<R: Read + Seek> Read for Timed<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.at >= self.len || buffer.is_empty() {
            // Its end (or a file grown since: read, but not as the disk's).
            let read = self.file.read(buffer)?;
            self.at += read as u64;
            return Ok(read);
        }
        let timer = self.reader.disk.timer.clone();
        let window = self.reader.window()?;
        let first = !self.second;
        if first && !self.begun {
            self.begun = true;
            window.timed_files += 1;
        }
        let started = timer.elapsed();
        let read = self.file.read(buffer)?;
        let took = timer.elapsed().saturating_sub(started);
        self.at += read as u64;
        window.took += took;
        window.bytes += read as u64;
        if first {
            window.timed += took;
            window.timed_bytes += read as u64;
        }
        if window.bytes >= WINDOW_BYTES || window.timed_files >= WINDOW_FILES {
            self.reader.end_window();
        }
        Ok(read)
    }
}

impl<R: Read + Seek> Seek for Timed<'_, R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let at = self.file.seek(position)?;
        if at == 0 && self.begun {
            self.second = true;
        }
        self.at = at;
        Ok(at)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
#[path = "disk_turns_tests.rs"]
mod tests;
