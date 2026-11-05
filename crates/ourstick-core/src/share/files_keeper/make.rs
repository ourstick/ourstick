//! Making one file ready (ADR 0017, rule 7): its plaintext hash and, if it
//! has more than one group, its outboard, read through its disk's turn and
//! kept only if its blob is still the version's (rule 4).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cipher::{Cipher, CipherError};
use crate::disk::turns::{DiskId, DiskTurns, Readers};
use crate::layout::StickLayout;
use crate::manifest::hashing::hash_file_telling_plaintext;
use crate::manifest::{FileStamp, ManifestError};
use crate::sender_files::SenderFiles;
use crate::store::OwnedFile;
use crate::transport::BlobHash;

/// One blob to make, at its paths in the version (identical files share a
/// blob: it is read once, at the first, and recorded at each).
#[derive(Debug, Clone)]
pub(super) struct ToMake {
    pub(super) hash: BlobHash,
    /// The index of its first entry in the version (ADR 0019).
    pub(super) index: u32,
    pub(super) paths: Vec<String>,
    /// For each path, the stamp the catalogue recorded with this blob.
    pub(super) stamps: Vec<Option<FileStamp>>,
}

/// What making a blob gave.
pub(super) enum Made {
    /// Its records (one per path), with the plaintext hash.
    Ready(Vec<OwnedFile>),
    /// Its file is no longer what the version lists.
    Changed,
    Unreadable(String),
}

/// Where a stick's files are: its roots, read through `files`.
pub(super) struct StickFiles<'a> {
    pub layout: &'a StickLayout,
    pub files: &'a dyn SenderFiles,
}

/// Makes `blob` ready. Stops between disk windows once `stop` is set (the
/// keeper ended). Blocking.
pub(super) fn make(
    blob: &ToMake,
    StickFiles { layout, files }: StickFiles<'_>,
    cipher: &dyn Cipher,
    outboards: &Path,
    disks: Arc<DiskTurns>,
    urgent: bool,
    stop: Arc<AtomicBool>,
) -> Made {
    let first = &blob.paths[0];
    let path = layout.on_disk(first);
    let facts = match files.facts_followed(&path) {
        Ok(facts) => facts,
        Err(error) => return Made::Unreadable(error.to_string()),
    };
    // Taken before the file is read: if it changes after, the next pass
    // reads it again.
    let stamp = facts.stamp;
    let mut readers = Readers::new(disks, urgent, move || {
        if stop.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("sharing stopped"));
        }
        Ok(())
    });
    let reader = readers.on(DiskId::of_facts(&path, &facts));
    let hashed = hash_file_telling_plaintext(
        files,
        first.clone(),
        &path,
        layout.names_below_root(first),
        cipher,
        Some(reader),
        Some(outboards),
    );
    readers.end_windows();
    let plaintext_hash = match hashed {
        Ok(hashed) if hashed.entry.hash == blob.hash => hashed.plaintext_hash,
        Ok(_)
        | Err(ManifestError::Blob {
            source: CipherError::PlaintextChanged,
            ..
        }) => return Made::Changed,
        Err(error) => return Made::Unreadable(error.to_string()),
    };
    let record = |path: &str, stamp| OwnedFile {
        path: path.to_owned(),
        stamp,
        blob_hash: blob.hash,
        plaintext_hash: Some(plaintext_hash),
    };
    let mut records = vec![record(first, stamp)];
    // The same content at other paths, recorded only while each is as the
    // catalogue found it (not read here: one changed since would be
    // recorded with a blob it no longer gives).
    for (other, recorded) in blob.paths.iter().zip(&blob.stamps).skip(1) {
        let Some(recorded) = recorded else { continue };
        if files
            .facts_followed(&layout.on_disk(other))
            .is_ok_and(|facts| facts.stamp == *recorded)
        {
            records.push(record(other, *recorded));
        }
    }
    Made::Ready(records)
}
