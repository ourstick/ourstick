//! A version as the owner's store serves it (ADR 0017): which of its files
//! can be served now, and which must be made first (rules 5 and 7).

use std::collections::HashMap;
use std::path::Path;

use bao_tree::BaoTree;

use super::super::ShareError;
use crate::blob::blob_len;
use crate::cipher::Cipher;
use crate::layout::StickLayout;
use crate::manifest::hashing::{outboard_path, BLOCK_SIZE};
use crate::manifest::{Entry, FileStamp};
use crate::stick_id::StickId;
use crate::store::Store;
use crate::transport::{BlobHash, PublishedVersion, ServedFile, ServedFiles};

/// The version being served.
pub(super) struct Version {
    pub(super) manifest_hash: BlobHash,
    /// Its files the store serves now, with its manifest (shared, ADR 0019).
    pub(super) served: ServedFiles,
    /// For each entry of the manifest, by index: what the keeper needs.
    pub(super) listed: Vec<Listed>,
}

/// What the keeper keeps of one entry of the version.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Listed {
    /// Whether the store serves it now (else it lacks its plaintext hash or
    /// its outboard).
    pub(super) served: bool,
    /// The stamp the catalogue recorded for it with this blob, if any.
    pub(super) stamp: Option<FileStamp>,
}

/// The version the owner published, `manifest_hash` with its `manifest`
/// as the publisher decoded it (ADR 0019), split into what can be served
/// now and what must be made. Blocking.
pub(super) fn load_version(
    store: &dyn Store,
    stick_id: &StickId,
    outboards: &Path,
    cipher: &dyn Cipher,
    published: PublishedVersion,
) -> Result<Version, ShareError> {
    let PublishedVersion {
        manifest_hash,
        manifest,
    } = published;
    // Read again with each version: linked entries come and go.
    let layout = StickLayout::load(store, stick_id)?;
    // What the store recorded of each entry's file, by the entry's index:
    // its rows walked one at a time, each found among the entries (sorted
    // by path), never all held at once. Nothing on disk is looked at
    // meanwhile: the store is held (ADR 0019).
    let mut recorded: Vec<Option<(Option<[u8; 32]>, FileStamp)>> =
        vec![None; manifest.entries.len()];
    store.each_owned_file(stick_id, &mut |file| {
        if let Ok(index) = manifest
            .entries
            .binary_search_by(|entry| entry.path.as_str().cmp(&file.path))
        {
            if manifest.entries[index].hash == file.blob_hash {
                recorded[index] = Some((file.plaintext_hash, file.stamp));
            }
        }
        std::ops::ControlFlow::Continue(())
    })?;
    let mut files = HashMap::with_capacity(manifest.entries.len());
    let mut listed = Vec::with_capacity(manifest.entries.len());
    for (index, (entry, record)) in manifest.entries.iter().zip(recorded).enumerate() {
        let stamp = record.map(|(_, stamp)| stamp);
        let served = match record.and_then(|(plaintext_hash, _)| plaintext_hash) {
            Some(plaintext_hash) if has_outboard(outboards, cipher, entry) => {
                // A blob at several paths is served from its first.
                files.entry(entry.hash).or_insert(ServedFile {
                    index: u32::try_from(index).expect("fewer than 2^32 entries"),
                    plaintext_hash,
                });
                true
            }
            _ => false,
        };
        listed.push(Listed { served, stamp });
    }
    Ok(Version {
        manifest_hash,
        served: ServedFiles {
            layout,
            manifest,
            files,
        },
        listed,
    })
}

/// Whether `entry`'s blob needs no outboard (one 16 KiB group) or has one
/// of the length its tree gives.
fn has_outboard(outboards: &Path, cipher: &dyn Cipher, entry: &Entry) -> bool {
    let tree = BaoTree::new(blob_len(cipher, entry.size), BLOCK_SIZE);
    tree.outboard_size() == 0
        || std::fs::metadata(outboard_path(outboards, &entry.hash))
            .is_ok_and(|metadata| metadata.len() == tree.outboard_size())
}

#[cfg(test)]
#[path = "version_tests.rs"]
mod tests;
