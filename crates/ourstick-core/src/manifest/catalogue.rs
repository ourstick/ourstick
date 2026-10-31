//! The owner's catalogue (ADR 0007): every file of the folder, in manifest
//! order, with the hash of the blob it would encrypt to, without storing that
//! blob. It is taken in parts, so the caller can record and publish what it
//! has between them.
//!
//! A file whose stamp is as recorded keeps its recorded hash without being
//! read (the heuristic of ADR 0005, "Known limits" in `docs/protocol.md`).
//! Any other file is encrypted in full, only to hash the blob: the hash then
//! is exactly what a later full encryption gives, if the file is unchanged.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use self::known::{carried, Known};
use super::hashing::{hash_file_telling_plaintext, Hashed};
use super::{Entry, FileStamp, ManifestError};
use crate::cipher::{Cipher, CipherError};
use crate::disk::turns::{DiskId, Readers};
use crate::layout::StickLayout;
use crate::sender_files::{LocalFiles, SenderFiles};
use crate::store::OwnedFile;

/// One file of the catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CataloguedFile {
    pub entry: Entry,
    /// Taken before the file was read: if it changes after, its stamp no
    /// longer matches and the next pass reads it again.
    pub stamp: FileStamp,
    /// `None` when the file was not read and its record has none.
    pub plaintext_hash: Option<[u8; 32]>,
    /// Whether the file was read (not taken from its record).
    pub read: bool,
}

impl CataloguedFile {
    /// The row to record for next time.
    pub fn owned_file(&self) -> OwnedFile {
        OwnedFile {
            path: self.entry.path.clone(),
            stamp: self.stamp,
            blob_hash: self.entry.hash,
            plaintext_hash: self.plaintext_hash,
        }
    }
}

/// What became of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Catalogued {
    File(CataloguedFile),
    /// Gone since the walk, or changing while it was read: left out of this
    /// pass. The caller runs another pass later.
    Skipped {
        path: String,
    },
}

/// Where a file of the catalogue comes from. Small, as there is one per
/// file (ADR 0007: a stick may have hundreds of thousands): the path on
/// disk is made from the root's when the file is read.
#[derive(Debug, Clone, Copy)]
pub(super) enum Source {
    /// A file on disk, under the root of this index in `roots`.
    Disk(u32),
    /// A file of a linked entry that is gone (ADR 0008): as recorded (in
    /// `known`), so the last version stays offered.
    Carried,
}

/// A root of the catalogue: where its files are on disk.
#[derive(Debug)]
pub(super) struct DiskRoot {
    path: PathBuf,
    /// The name of a linked entry, whose files' paths begin with it; none
    /// for the stick's own folder.
    name: Option<String>,
    is_dir: bool,
}

/// A pass over a stick's roots, taken in parts.
#[derive(Debug)]
pub struct Catalogue {
    /// Through what the files are read.
    sender_files: Arc<dyn SenderFiles>,
    /// Every file, sorted by its path as in the manifest (not in walk
    /// order: "a-c" comes before "a/b"), so each part extends a prefix of
    /// the manifest.
    files: Vec<(String, Source)>,
    roots: Vec<DiskRoot>,
    next: usize,
    /// What was recorded before, also with `rescan` (for files carried,
    /// and for [`Catalogue::recorded_entries`]).
    known: HashMap<String, Known>,
    rescan: bool,
    /// Where the outboards of files it reads go (ADR 0017), if anywhere.
    outboards: Option<PathBuf>,
    gone: Vec<String>,
    left_out: Vec<String>,
}

impl Catalogue {
    /// Walks the stick's own folder only. `known` is what was recorded
    /// before. Blocking.
    pub fn start(root: &Path, known: Vec<OwnedFile>) -> Result<Self, ManifestError> {
        let files = Arc::new(LocalFiles);
        Self::start_layout(files, &StickLayout::new(root, &[]), known, false)
    }

    /// Walks every root of `layout`. `recorded` is what was recorded before;
    /// with `rescan`, it is used only for linked entries that are gone (their
    /// files are carried as recorded), never to skip reading. Blocking.
    pub fn start_layout(
        sender_files: Arc<dyn SenderFiles>,
        layout: &StickLayout,
        recorded: Vec<OwnedFile>,
        rescan: bool,
    ) -> Result<Self, ManifestError> {
        let walk::Walked {
            mut files,
            roots,
            gone,
            left_out,
        } = walk::walk(sender_files.as_ref(), layout, &recorded)?;
        files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        files.shrink_to_fit();
        let known = recorded
            .into_iter()
            .map(|file| {
                let known = Known {
                    stamp: file.stamp,
                    blob_hash: file.blob_hash,
                    plaintext_hash: file.plaintext_hash,
                };
                (file.path, known)
            })
            .collect();
        Ok(Self {
            sender_files,
            files,
            roots,
            next: 0,
            known,
            rescan,
            outboards: None,
            gone,
            left_out,
        })
    }

    /// Keeps the outboard of each blob of more than one 16 KiB group it
    /// reads in `dir` (ADR 0017, rule 1).
    pub fn keep_outboards_in(mut self, dir: PathBuf) -> Self {
        self.outboards = Some(dir);
        self
    }

    /// What was recorded of the files the walk found, in manifest order, as
    /// the last version listed them (ADR 0007's partial versions keep these
    /// for files not read yet).
    pub fn recorded_entries(&self) -> Vec<Entry> {
        self.files
            .iter()
            .filter_map(|(path, _)| {
                self.known.get(path).map(|known| Entry {
                    path: path.clone(),
                    size: known.stamp.size,
                    // Whole seconds, as the catalogue records them.
                    mtime: known.stamp.mtime_ns / 1_000_000_000,
                    hash: known.blob_hash,
                })
            })
            .collect()
    }

    /// Where the file `relative`, under the root `index`, is on disk.
    fn on_disk(&self, index: u32, relative: &str) -> PathBuf {
        let root = &self.roots[index as usize];
        match &root.name {
            _ if !root.is_dir => root.path.clone(),
            None => {
                let mut path = root.path.clone();
                path.extend(relative.split('/'));
                path
            }
            // Part by part, so the separators are the system's.
            Some(name) => {
                let mut path = root.path.clone();
                path.extend(relative[name.len() + 1..].split('/'));
                path
            }
        }
    }

    /// The linked entries that are gone, by name: their files are carried
    /// as recorded.
    pub fn gone(&self) -> &[String] {
        &self.gone
    }

    /// Top-level names of the own folder left out, because a linked entry
    /// has that name (ignoring case).
    pub fn left_out(&self) -> &[String] {
        &self.left_out
    }

    /// How many files the walk found.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// How many files are done.
    pub fn done(&self) -> usize {
        self.next
    }

    pub fn is_finished(&self) -> bool {
        self.next == self.files.len()
    }

    /// The path of the last file done (catalogued or skipped).
    pub fn last_done(&self) -> Option<&str> {
        self.next
            .checked_sub(1)
            .map(|index| self.files[index].0.as_str())
    }

    /// Every path the walk found, in manifest order.
    pub fn paths(&self) -> Vec<String> {
        self.files.iter().map(|(path, _)| path.clone()).collect()
    }

    /// The next files, up to `max_files` of them, stopping once `max_bytes`
    /// were read. With `readers`, files are read through their disks' turns
    /// (ADR 0013), every turn given back before it returns. Blocking.
    pub fn next_part(
        &mut self,
        cipher: &dyn Cipher,
        max_files: usize,
        max_bytes: u64,
        mut readers: Option<&mut Readers>,
    ) -> Result<Vec<Catalogued>, ManifestError> {
        let part = self.read_part(cipher, max_files, max_bytes, readers.as_deref_mut());
        if let Some(readers) = readers {
            readers.end_windows();
        }
        part
    }

    fn read_part(
        &mut self,
        cipher: &dyn Cipher,
        max_files: usize,
        max_bytes: u64,
        mut readers: Option<&mut Readers>,
    ) -> Result<Vec<Catalogued>, ManifestError> {
        let mut part = Vec::new();
        let mut read = 0u64;
        while part.len() < max_files && read < max_bytes && !self.is_finished() {
            let (relative, source) = &self.files[self.next];
            let catalogued = match *source {
                Source::Disk(index) => {
                    let path = self.on_disk(index, relative);
                    self.catalogue(relative, &path, cipher, readers.as_deref_mut())?
                }
                Source::Carried => match self.known.get(relative) {
                    Some(known) => Catalogued::File(carried(relative, known)),
                    None => Catalogued::Skipped {
                        path: relative.clone(),
                    },
                },
            };
            if let Catalogued::File(file) = &catalogued {
                if file.read {
                    read += file.entry.size;
                }
            }
            part.push(catalogued);
            self.next += 1;
        }
        Ok(part)
    }

    fn catalogue(
        &self,
        relative: &str,
        path: &Path,
        cipher: &dyn Cipher,
        readers: Option<&mut Readers>,
    ) -> Result<Catalogued, ManifestError> {
        let skipped = || Catalogued::Skipped {
            path: relative.to_owned(),
        };
        let facts = match self.sender_files.facts(path) {
            Ok(facts) if facts.is_file() => facts,
            // Gone, or no longer a regular file (e.g. now a link).
            Ok(_) => return Ok(skipped()),
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(skipped()),
            Err(error) => {
                return Err(ManifestError::Io {
                    path: path.to_path_buf(),
                    source: error,
                })
            }
        };
        let stamp = facts.stamp;
        if let Some(known) = self
            .known
            .get(relative)
            .filter(|known| !self.rescan && known.stamp == stamp)
        {
            return Ok(Catalogued::File(CataloguedFile {
                entry: Entry {
                    path: relative.to_owned(),
                    size: stamp.size,
                    mtime: facts.mtime_seconds(),
                    hash: known.blob_hash,
                },
                stamp,
                plaintext_hash: known.plaintext_hash,
                read: false,
            }));
        }
        let reader = readers.map(|readers| readers.on(DiskId::of_facts(path, &facts)));
        let outboards = self.outboards.as_deref();
        match hash_file_telling_plaintext(relative.to_owned(), path, cipher, reader, outboards) {
            Ok(Hashed {
                entry,
                plaintext_hash,
            }) => Ok(Catalogued::File(CataloguedFile {
                entry,
                stamp,
                plaintext_hash: Some(plaintext_hash),
                read: true,
            })),
            Err(ManifestError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
                Ok(skipped())
            }
            Err(ManifestError::Blob {
                source: CipherError::PlaintextChanged,
                ..
            }) => Ok(skipped()),
            Err(error) => Err(error),
        }
    }
}

mod known;
mod walk;

#[cfg(test)]
mod tests;
#[cfg(test)]
#[path = "catalogue/tests_linked.rs"]
mod tests_linked;
