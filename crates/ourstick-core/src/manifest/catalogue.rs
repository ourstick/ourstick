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

/// A pass over one folder, taken in parts.
#[derive(Debug)]
pub struct Catalogue {
    root: PathBuf,
    /// Every file, sorted by its path as in the manifest (not in walk
    /// order: "a-c" comes before "a/b"), so each part extends a prefix of
    /// the manifest.
    files: Vec<(String, PathBuf)>,
    next: usize,
    known: HashMap<String, OwnedFile>,
}

impl Catalogue {
    /// Walks `root`. `known` is what was recorded before. Blocking.
    pub fn start(root: &Path, known: Vec<OwnedFile>) -> Result<Self, ManifestError> {
        let mut files = regular_files(root)?
            .into_iter()
            .map(|path| Ok((relative_path(root, &path)?, path)))
            .collect::<Result<Vec<_>, ManifestError>>()?;
        files.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Self {
            root: root.to_path_buf(),
            files,
            next: 0,
            known: known
                .into_iter()
                .map(|file| (file.path.clone(), file))
                .collect(),
        })
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
            let (relative, path) = &self.files[self.next];
            let catalogued = self.catalogue(relative, path, cipher)?;
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
        match hash_file_telling_plaintext(&self.root, path, cipher, None) {
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
