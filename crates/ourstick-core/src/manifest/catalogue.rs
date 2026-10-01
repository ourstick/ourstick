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
use std::time::UNIX_EPOCH;

use super::build::{hash_file_telling_plaintext, regular_files, relative_path};
use super::{Entry, FileStamp, ManifestError};
use crate::cipher::{Cipher, CipherError};
use crate::layout::StickLayout;
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

/// Where a file of the catalogue comes from.
#[derive(Debug)]
enum Source {
    /// A file on disk.
    Disk(PathBuf),
    /// A file of a linked entry that is gone (ADR 0008): as recorded, so
    /// the last version stays offered.
    Carried(OwnedFile),
}

/// A pass over a stick's roots, taken in parts.
#[derive(Debug)]
pub struct Catalogue {
    /// Every file, sorted by its path as in the manifest (not in walk
    /// order: "a-c" comes before "a/b"), so each part extends a prefix of
    /// the manifest.
    files: Vec<(String, Source)>,
    next: usize,
    known: HashMap<String, OwnedFile>,
    gone: Vec<String>,
    left_out: Vec<String>,
}

impl Catalogue {
    /// Walks the stick's own folder only. `known` is what was recorded
    /// before. Blocking.
    pub fn start(root: &Path, known: Vec<OwnedFile>) -> Result<Self, ManifestError> {
        Self::start_layout(&StickLayout::new(root, &[]), known, false)
    }

    /// Walks every root of `layout`. `recorded` is what was recorded before;
    /// with `rescan`, it is used only for linked entries that are gone (their
    /// files are carried as recorded), never to skip reading. Blocking.
    pub fn start_layout(
        layout: &StickLayout,
        recorded: Vec<OwnedFile>,
        rescan: bool,
    ) -> Result<Self, ManifestError> {
        let own = &layout.own().path;
        let mut files = Vec::new();
        let mut left_out = std::collections::BTreeSet::new();
        for path in regular_files(own)? {
            let relative = relative_path(own, &path)?;
            let top = relative.split('/').next().unwrap_or_default();
            if layout.is_shadowed(top) {
                left_out.insert(top.to_owned());
                continue;
            }
            files.push((relative, Source::Disk(path)));
        }
        let mut gone = Vec::new();
        for root in layout.linked() {
            let name = root.name.clone().unwrap_or_default();
            if root.is_gone() {
                let prefix = format!("{name}/");
                files.extend(
                    recorded
                        .iter()
                        .filter(|file| file.path == name || file.path.starts_with(&prefix))
                        .map(|file| (file.path.clone(), Source::Carried(file.clone()))),
                );
                gone.push(name);
            } else if root.is_dir {
                for path in regular_files(&root.path)? {
                    let relative = format!("{name}/{}", relative_path(&root.path, &path)?);
                    files.push((relative, Source::Disk(path)));
                }
            } else {
                files.push((name, Source::Disk(root.path.clone())));
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let known = if rescan { Vec::new() } else { recorded };
        Ok(Self {
            files,
            next: 0,
            known: known
                .into_iter()
                .map(|file| (file.path.clone(), file))
                .collect(),
            gone,
            left_out: left_out.into_iter().collect(),
        })
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
    /// were read. Blocking.
    pub fn next_part(
        &mut self,
        cipher: &dyn Cipher,
        max_files: usize,
        max_bytes: u64,
    ) -> Result<Vec<Catalogued>, ManifestError> {
        let mut part = Vec::new();
        let mut read = 0u64;
        while part.len() < max_files && read < max_bytes && !self.is_finished() {
            let (relative, source) = &self.files[self.next];
            let catalogued = match source {
                Source::Disk(path) => self.catalogue(relative, path, cipher)?,
                Source::Carried(file) => Catalogued::File(carried(file)),
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
    ) -> Result<Catalogued, ManifestError> {
        let skipped = || Catalogued::Skipped {
            path: relative.to_owned(),
        };
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => metadata,
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
        let stamp = FileStamp::of(&metadata);
        if let Some(known) = self
            .known
            .get(relative)
            .filter(|known| known.stamp == stamp)
        {
            return Ok(Catalogued::File(CataloguedFile {
                entry: Entry {
                    path: relative.to_owned(),
                    size: stamp.size,
                    mtime: mtime_seconds(&metadata),
                    hash: known.blob_hash,
                },
                stamp,
                plaintext_hash: known.plaintext_hash,
                read: false,
            }));
        }
        match hash_file_telling_plaintext(relative.to_owned(), path, cipher, None) {
            Ok((entry, _, plaintext_hash)) => Ok(Catalogued::File(CataloguedFile {
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

/// A recorded file, unread, as the manifest had it.
fn carried(file: &OwnedFile) -> CataloguedFile {
    CataloguedFile {
        entry: Entry {
            path: file.path.clone(),
            size: file.stamp.size,
            mtime: file.stamp.mtime_ns / 1_000_000_000,
            hash: file.blob_hash,
        },
        stamp: file.stamp,
        plaintext_hash: file.plaintext_hash,
        read: false,
    }
}

/// The manifest's mtime: whole seconds, as `hash_file` records it.
fn mtime_seconds(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
#[path = "catalogue/tests_linked.rs"]
mod tests_linked;
