//! [`SenderFiles`] kept in memory under a made-up root, as a phone's app
//! answers for the paths it gave out: nothing under that root is on disk.
//! Any other path is read from the file system (a stick's own folder is
//! always a real one). For tests.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{FileFacts, FileKind, LocalFiles, ReadAt, SenderFiles};
use crate::manifest::FileStamp;

/// A file's contents and when it was written, in nanoseconds.
type Written = (Arc<Vec<u8>>, u64);

/// Files in memory under `root`.
#[derive(Debug)]
pub struct MemoryFiles {
    root: PathBuf,
    /// Each file by its path under `root`, with its contents and when it
    /// was last written (in nanoseconds).
    files: Mutex<BTreeMap<PathBuf, Written>>,
    /// Writes so far, for each write's time.
    writes: Mutex<u64>,
}

impl MemoryFiles {
    /// No files yet, under `root` (which nothing on disk should have).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            files: Mutex::default(),
            writes: Mutex::default(),
        }
    }

    /// The path of `relative` (`/`-separated) under the root.
    pub fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.root.clone(), |path, part| path.join(part))
    }

    /// Writes `contents` at `relative`, a second later than the last write.
    pub fn write(&self, relative: &str, contents: &[u8]) {
        let mut writes = lock(&self.writes);
        *writes += 1;
        let time = 1_700_000_000_000_000_000 + *writes * 1_000_000_000;
        lock(&self.files).insert(self.path(relative), (Arc::new(contents.to_vec()), time));
    }

    /// Writes `contents` at `relative` with no time told, as some phone
    /// pickers' files are.
    pub fn write_untimed(&self, relative: &str, contents: &[u8]) {
        lock(&self.files).insert(self.path(relative), (Arc::new(contents.to_vec()), 0));
    }

    /// Removes the file at `relative`.
    pub fn remove(&self, relative: &str) {
        lock(&self.files).remove(&self.path(relative));
    }

    fn is_ours(&self, path: &Path) -> bool {
        path.starts_with(&self.root)
    }

    fn ours(&self, path: &Path) -> io::Result<FileFacts> {
        let files = lock(&self.files);
        if let Some((contents, time)) = files.get(path) {
            return Ok(FileFacts {
                kind: FileKind::File,
                stamp: stamp(contents.len() as u64, *time),
                disk: None,
                time_known: *time != 0,
            });
        }
        if path == self.root || files.keys().any(|file| file.starts_with(path)) {
            return Ok(FileFacts {
                kind: FileKind::Folder,
                stamp: stamp(0, 0),
                disk: None,
                time_known: true,
            });
        }
        Err(io::ErrorKind::NotFound.into())
    }
}

fn stamp(size: u64, mtime_ns: u64) -> FileStamp {
    FileStamp {
        size,
        mtime_ns,
        ctime_ns: 0,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl SenderFiles for MemoryFiles {
    fn facts(&self, path: &Path) -> io::Result<FileFacts> {
        if self.is_ours(path) {
            self.ours(path)
        } else {
            LocalFiles.facts(path)
        }
    }

    fn facts_followed(&self, path: &Path) -> io::Result<FileFacts> {
        if self.is_ours(path) {
            self.ours(path)
        } else {
            LocalFiles.facts_followed(path)
        }
    }

    fn entries(&self, path: &Path) -> io::Result<Vec<(OsString, FileFacts)>> {
        if !self.is_ours(path) {
            return LocalFiles.entries(path);
        }
        if !self.ours(path)?.is_folder() {
            return Err(io::Error::other("not a folder"));
        }
        let names: Vec<OsString> = {
            let files = lock(&self.files);
            let mut names: Vec<OsString> = files
                .keys()
                .filter_map(|file| file.strip_prefix(path).ok())
                .filter_map(|inside| inside.components().next())
                .map(|first| first.as_os_str().to_owned())
                .collect();
            names.dedup();
            names
        };
        names
            .into_iter()
            .map(|name| {
                let facts = self.ours(&path.join(&name))?;
                Ok((name, facts))
            })
            .collect()
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn ReadAt>> {
        if !self.is_ours(path) {
            return LocalFiles.open(path);
        }
        let (contents, time) = lock(&self.files)
            .get(path)
            .cloned()
            .ok_or(io::ErrorKind::NotFound)?;
        Ok(Box::new(Opened { contents, time }))
    }
}

/// A file in memory as it was when opened.
struct Opened {
    contents: Arc<Vec<u8>>,
    time: u64,
}

impl ReadAt for Opened {
    fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(self.contents.len());
        let n = buffer.len().min(self.contents.len() - start);
        buffer[..n].copy_from_slice(&self.contents[start..start + n]);
        Ok(n)
    }

    fn facts(&self) -> io::Result<FileFacts> {
        Ok(FileFacts {
            kind: FileKind::File,
            stamp: stamp(self.contents.len() as u64, self.time),
            disk: None,
            time_known: self.time != 0,
        })
    }
}
