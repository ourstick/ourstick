//! A version as the owner's store serves it (ADR 0017): which of its files
//! can be served now, and which must be made first (rules 5 and 7).

use std::collections::HashMap;
use std::path::Path;

use bao_tree::BaoTree;

use super::super::latest::load_latest_manifest;
use super::super::ShareError;
use crate::blob::{blob_len, decrypt_blob};
use crate::cipher::Cipher;
use crate::layout::StickLayout;
use crate::manifest::hashing::{outboard_path, BLOCK_SIZE};
use crate::manifest::{Entry, FileStamp, Manifest};
use crate::stick_id::StickId;
use crate::store::Store;
use crate::transport::{BlobHash, ServedFile, ServedFiles};

/// The version being served.
pub(super) struct Version {
    pub(super) manifest_hash: BlobHash,
    /// Its files the store serves now.
    pub(super) served: ServedFiles,
    /// Every file of the version, in manifest order.
    pub(super) entries: Vec<Listed>,
}

/// A file of the version.
pub(super) struct Listed {
    pub(super) entry: Entry,
    /// Whether the store serves it now (else it lacks its plaintext hash or
    /// its outboard).
    pub(super) served: bool,
    /// The stamp the catalogue recorded for it with this blob, if any.
    pub(super) stamp: Option<FileStamp>,
}

/// The latest version the owner published, split into what can be served
/// now and what must be made; `None` before the first. Blocking.
pub(super) fn load_version(
    store: &dyn Store,
    stick_id: &StickId,
    cache_dir: &Path,
    outboards: &Path,
    cipher: &dyn Cipher,
) -> Result<Option<Version>, ShareError> {
    let owned = store.owned_stick(stick_id)?.ok_or(ShareError::NotOwned)?;
    let Some(record) = owned.latest_record else {
        return Ok(None);
    };
    let manifest_hash = record.unverified_record().manifest_hash;
    let Some(blob) = load_latest_manifest(cache_dir, &manifest_hash) else {
        return Ok(None);
    };
    let mut plaintext = Vec::new();
    decrypt_blob(cipher, &mut blob.as_slice(), &mut plaintext)?;
    let manifest = Manifest::from_bytes(&plaintext)?;
    // Read again with each version: linked entries come and go.
    let layout = StickLayout::load(store, stick_id)?;
    let mut recorded = store.owned_files(stick_id)?;
    recorded.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    let mut files = HashMap::with_capacity(manifest.entries.len());
    let mut entries = Vec::with_capacity(manifest.entries.len());
    for entry in manifest.entries {
        let record = recorded
            .binary_search_by(|file| file.path.as_str().cmp(&entry.path))
            .ok()
            .map(|index| &recorded[index])
            .filter(|file| file.blob_hash == entry.hash);
        let stamp = record.map(|file| file.stamp);
        match record.and_then(|file| file.plaintext_hash) {
            Some(plaintext_hash) if has_outboard(outboards, cipher, &entry) => {
                files.insert(
                    entry.hash,
                    ServedFile {
                        path: entry.path.as_str().into(),
                        plaintext_len: entry.size,
                        plaintext_hash,
                    },
                );
                entries.push(Listed {
                    entry,
                    served: true,
                    stamp,
                });
            }
            _ => entries.push(Listed {
                entry,
                served: false,
                stamp,
            }),
        }
    }
    Ok(Some(Version {
        manifest_hash,
        served: ServedFiles { layout, files },
        entries,
    }))
}

/// Whether `entry`'s blob needs no outboard (one 16 KiB group) or has one
/// of the length its tree gives.
fn has_outboard(outboards: &Path, cipher: &dyn Cipher, entry: &Entry) -> bool {
    let tree = BaoTree::new(blob_len(cipher, entry.size), BLOCK_SIZE);
    tree.outboard_size() == 0
        || std::fs::metadata(outboard_path(outboards, &entry.hash))
            .is_ok_and(|metadata| metadata.len() == tree.outboard_size())
}
