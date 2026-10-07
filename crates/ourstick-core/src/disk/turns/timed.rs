//! Reading files through a disk's turns: a [`DiskReader`] (a pass, a file
//! made ready) holds a turn for one window at a time, and [`Timed`] times the
//! first read of each file into it (ADR 0013, rules 1 and 2).

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

use super::{Disk, DiskId, DiskTurns};
use crate::disk::counters::Counters;
use crate::disk::share::{ReaderKind, Seen, WINDOW_BYTES, WINDOW_FILES};

/// What one turn has read.
#[derive(Debug, Default)]
pub(super) struct Window {
    /// Whose window it is: each kind's speed is learned apart.
    pub(super) kind: ReaderKind,
    /// Files begun and their first reads' bytes and time: what is judged.
    pub(super) timed_files: u64,
    pub(super) timed_bytes: u64,
    pub(super) timed: Duration,
    /// Every read's time, the second reads' too: what is paused for.
    pub(super) took: Duration,
    pub(super) bytes: u64,
    /// Read calls made in it.
    pub(super) reads: u64,
    /// What the kernel counted on the disk while its reads ran (ADR 0013,
    /// amendment 1): summed over its spans of reading, from `since`.
    pub(super) disk: Counters,
    pub(super) since: Option<Counters>,
    pub(super) spans: u32,
    /// A count could not be read: judged by timing alone.
    pub(super) uncounted: bool,
}

impl Window {
    /// A span of reading begins; `now` is the disk's count, if any.
    pub(super) fn count_from(&mut self, now: Option<Counters>) {
        match now {
            Some(now) => self.since = Some(now),
            None => self.uncounted = true,
        }
    }

    /// A span of reading ends.
    pub(super) fn count_to(&mut self, now: Option<Counters>) {
        match (self.since.take(), now) {
            (Some(since), Some(now)) => {
                self.disk.add(since.until(now));
                self.spans += 1;
            }
            _ => self.uncounted = true,
        }
    }

    /// What the kernel saw of it, where it could count every span.
    pub(super) fn seen(&self) -> Option<Seen> {
        (!self.uncounted && self.spans > 0).then_some(Seen {
            disk: self.disk,
            ours: self.bytes,
            reads: self.reads,
        })
    }
}

/// One reader of a disk (a catalogue pass, a file made ready): its turn
/// and its current window. Files are read through [`DiskReader::open`].
pub struct DiskReader {
    disk: Arc<Disk>,
    urgent: bool,
    go_on: Box<dyn Fn() -> io::Result<()> + Send>,
    window: Option<Window>,
}

impl DiskReader {
    /// `urgent`: a file that readers wait for. `go_on` is asked before
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

    /// Whether this reader's next turns are urgent (readers wait for it).
    pub fn set_urgent(&mut self, urgent: bool) {
        self.urgent = urgent;
    }

    /// Gives the turn back now (the end of a part), judging what was read.
    pub fn end_window(&mut self) {
        if let Some(mut window) = self.window.take() {
            window.count_to(self.disk.count());
            self.disk.give_back(Some(&window));
        }
    }

    /// The window to read into, taking a turn first if none is held.
    fn window(&mut self) -> io::Result<&mut Window> {
        if self.window.is_none() {
            self.disk.take_turn(self.urgent, self.go_on.as_ref())?;
            let mut window = Window::default();
            window.count_from(self.disk.count());
            self.window = Some(window);
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
            // Its end: no turn needed to learn it.
            let read = self.file.read(buffer)?;
            self.at += read as u64;
            if read > 0 {
                // It grew since its length was taken (a video being
                // recorded into the folder): from now on read in turns.
                self.len = u64::MAX;
            }
            return Ok(read);
        }
        let first = !self.second;
        let begins = first && !self.begun;
        // A window holds at most 64 files: the 65th begins the next one.
        if begins
            && self
                .reader
                .window
                .as_ref()
                .is_some_and(|window| window.timed_files >= WINDOW_FILES)
        {
            self.reader.end_window();
        }
        let timer = self.reader.disk.timer.clone();
        let window = self.reader.window()?;
        if begins {
            self.begun = true;
            window.timed_files += 1;
        }
        let started = timer.elapsed();
        let read = self.file.read(buffer)?;
        let took = timer.elapsed().saturating_sub(started);
        window.reads += 1;
        self.at += read as u64;
        window.took += took;
        window.bytes += read as u64;
        if first {
            window.timed += took;
            window.timed_bytes += read as u64;
        }
        if window.bytes >= WINDOW_BYTES {
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

/// Whether to go on reading, shared by every reader of one task.
type GoOn = Arc<dyn Fn() -> io::Result<()> + Send + Sync>;

/// The readers of one task (a part of a catalogue) over every disk it
/// reads: one [`DiskReader`] per disk, and one turn held at a time, so a
/// task never waits for a disk while it holds another's turn.
pub struct Readers {
    disks: Arc<DiskTurns>,
    urgent: bool,
    go_on: GoOn,
    readers: HashMap<DiskId, DiskReader>,
    current: Option<DiskId>,
}

impl Readers {
    /// See [`DiskReader::new`] for `urgent` and `go_on`.
    pub fn new(
        disks: Arc<DiskTurns>,
        urgent: bool,
        go_on: impl Fn() -> io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            disks,
            urgent,
            go_on: Arc::new(go_on),
            readers: HashMap::new(),
            current: None,
        }
    }

    /// The reader of the disk `id`; another disk's turn is given back
    /// first.
    pub fn on(&mut self, id: DiskId) -> &mut DiskReader {
        if self.current.as_ref() != Some(&id) {
            if let Some(reader) = self
                .current
                .take()
                .and_then(|current| self.readers.get_mut(&current))
            {
                reader.end_window();
            }
            self.current = Some(id.clone());
        }
        let disks = &self.disks;
        let urgent = self.urgent;
        let go_on = self.go_on.clone();
        self.readers
            .entry(id.clone())
            .or_insert_with(|| DiskReader::new(disks.get(id), urgent, move || go_on()))
    }

    /// Whether the next turns are urgent, for every disk.
    pub fn set_urgent(&mut self, urgent: bool) {
        self.urgent = urgent;
        for reader in self.readers.values_mut() {
            reader.set_urgent(urgent);
        }
    }

    /// Gives back every turn held (the end of a part).
    pub fn end_windows(&mut self) {
        for reader in self.readers.values_mut() {
            reader.end_window();
        }
        self.current = None;
    }

    /// Whether OurStick gives way on any disk these readers read.
    pub fn giving_way(&self) -> bool {
        self.readers.values().any(|reader| reader.disk.giving_way())
    }
}

impl std::fmt::Debug for Readers {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Readers")
            .field("disks", &self.readers.len())
            .finish_non_exhaustive()
    }
}
