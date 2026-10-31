//! [`SenderFiles`] on the file system, by path.

use std::ffi::OsString;
use std::fs::{File, Metadata};
use std::io;
use std::path::Path;

use super::{FileFacts, FileKind, ReadAt, SenderFiles};
use crate::linked::disk_of;
use crate::manifest::FileStamp;

/// The file system, read by path with `std::fs`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalFiles;

impl FileFacts {
    /// The facts `metadata` tells.
    pub fn of(metadata: &Metadata) -> Self {
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            FileKind::Link
        } else if file_type.is_dir() {
            FileKind::Folder
        } else if file_type.is_file() {
            FileKind::File
        } else {
            FileKind::Other
        };
        Self {
            kind,
            stamp: FileStamp::of(metadata),
            disk: disk_of(metadata),
        }
    }
}

impl SenderFiles for LocalFiles {
    fn facts(&self, path: &Path) -> io::Result<FileFacts> {
        Ok(FileFacts::of(&std::fs::symlink_metadata(path)?))
    }

    fn facts_followed(&self, path: &Path) -> io::Result<FileFacts> {
        Ok(FileFacts::of(&std::fs::metadata(path)?))
    }

    fn entries(&self, path: &Path) -> io::Result<Vec<(OsString, FileFacts)>> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            // `DirEntry::metadata` does not follow a link.
            entries.push((entry.file_name(), FileFacts::of(&entry.metadata()?)));
        }
        Ok(entries)
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn ReadAt>> {
        Ok(Box::new(File::open(path)?))
    }
}

impl ReadAt for File {
    #[cfg(unix)]
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        std::os::unix::fs::FileExt::read_at(self, buffer, offset)
    }

    /// Moves the file's cursor too, which no one here uses.
    #[cfg(windows)]
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        std::os::windows::fs::FileExt::seek_read(self, buffer, offset)
    }

    fn facts(&self) -> io::Result<FileFacts> {
        Ok(FileFacts::of(&self.metadata()?))
    }
}
