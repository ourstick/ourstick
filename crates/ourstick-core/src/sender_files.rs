//! The sender's files (ADR 0029): what the core reads of the files a stick
//! shares, behind one trait (rule 2). [`LocalFiles`] reads the file system
//! by path, as every desktop does and the tests do; a phone's app gives its
//! own, which reads what the user chose through the system's pickers and
//! answers for the paths it gave out.
//!
//! An item is named by a path: a root the store keeps, joined with a
//! manifest path's parts. Every method blocks: call it from the blocking
//! pool.

use std::ffi::OsString;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use crate::manifest::FileStamp;

mod local;
mod memory;
mod walk;

pub use local::LocalFiles;
pub use memory::MemoryFiles;
pub use walk::for_each_file;

/// What an item is. A symbolic link is never followed by the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Folder,
    Link,
    /// A socket, a device, a pipe: never shared.
    Other,
}

/// What is known of an item without reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFacts {
    pub kind: FileKind,
    /// Its size and times; a folder's size means nothing.
    pub stamp: FileStamp,
    /// The disk it is on (`st_dev`), where the system tells.
    pub disk: Option<u64>,
    /// Whether its time is told. One that is not (some phone pickers'
    /// files) has a stamp that cannot show a change of the same size: it is
    /// read again at every pass, never taken from its record.
    pub time_known: bool,
}

impl FileFacts {
    /// The manifest's mtime: whole seconds.
    pub fn mtime_seconds(&self) -> u64 {
        self.stamp.mtime_ns / 1_000_000_000
    }

    pub fn is_file(&self) -> bool {
        self.kind == FileKind::File
    }

    pub fn is_folder(&self) -> bool {
        self.kind == FileKind::Folder
    }
}

/// A file opened for reading by position.
pub trait ReadAt: Send + Sync {
    /// Reads into `buffer` from `offset`; how many bytes, 0 at the end.
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize>;

    /// Fills `buffer` from `offset`, or fails with `UnexpectedEof`.
    fn read_exact_at(&self, mut buffer: &mut [u8], mut offset: u64) -> io::Result<()> {
        while !buffer.is_empty() {
            match self.read_at(buffer, offset) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the file is shorter than recorded",
                    ))
                }
                Ok(n) => {
                    buffer = &mut buffer[n..];
                    offset += n as u64;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Its facts now (a file opened stays the same file, also if renamed).
    fn facts(&self) -> io::Result<FileFacts>;
}

/// The files a stick shares, read where they are.
pub trait SenderFiles: Send + Sync + std::fmt::Debug {
    /// The item at `path`, a final symbolic link not followed.
    fn facts(&self, path: &Path) -> io::Result<FileFacts>;

    /// The item at `path`, symbolic links followed.
    fn facts_followed(&self, path: &Path) -> io::Result<FileFacts>;

    /// The entries of the folder at `path`, in no order, links not followed.
    fn entries(&self, path: &Path) -> io::Result<Vec<(OsString, FileFacts)>>;

    /// The file at `path`, for reading.
    fn open(&self, path: &Path) -> io::Result<Box<dyn ReadAt>>;

    /// How the place `a` relates to the place `b` (ADR 0029): the same,
    /// one holding the other, or apart (`None`). Compared as paths here;
    /// a phone's places are not paths and cannot be compared as text.
    fn relation(&self, a: &Path, b: &Path) -> Option<crate::places::Relation> {
        crate::places::relation(&crate::places::comparable(a), &crate::places::comparable(b))
    }
}

/// A [`ReadAt`] read in order, as [`Read`] and [`Seek`] (the catalogue
/// hashes a file in order).
pub struct InOrder {
    file: Box<dyn ReadAt>,
    position: u64,
}

impl InOrder {
    pub fn new(file: Box<dyn ReadAt>) -> Self {
        Self { file, position: 0 }
    }
}

impl Read for InOrder {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let n = self.file.read_at(buffer, self.position)?;
        self.position += n as u64;
        Ok(n)
    }
}

impl Seek for InOrder {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "seek before the start");
        self.position = match to {
            SeekFrom::Start(at) => at,
            SeekFrom::Current(by) => self.position.checked_add_signed(by).ok_or_else(invalid)?,
            SeekFrom::End(by) => self
                .file
                .facts()?
                .stamp
                .size
                .checked_add_signed(by)
                .ok_or_else(invalid)?,
        };
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests;
