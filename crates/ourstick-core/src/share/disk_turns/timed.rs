//! Reading files through a disk's turns: a [`DiskReader`] (a pass, a
//! suitcase) holds a turn for one window at a time, and [`Timed`] times the
//! first read of each file into it (ADR 0013, rules 1 and 2).

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

use super::Disk;
use crate::share::disk_share::{WINDOW_BYTES, WINDOW_FILES};

/// What one turn has read.
#[derive(Debug, Default)]
pub(super) struct Window {
    /// Files begun and their first reads' bytes and time: what is judged.
    pub(super) timed_files: u64,
    pub(super) timed_bytes: u64,
    pub(super) timed: Duration,
    /// Every read's time, the second reads' too: what is paused for.
    pub(super) took: Duration,
    pub(super) bytes: u64,
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
