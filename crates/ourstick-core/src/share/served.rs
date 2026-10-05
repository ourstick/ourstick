//! What the owner's store serves for a version (ADR 0017): each file of the
//! manifest that the catalogue recorded with a plaintext hash, by blob hash,
//! with the stick's layout to find it on disk.

use std::collections::HashMap;

use crate::layout::StickLayout;
use crate::manifest::Entry;
use crate::stick_id::StickId;
use crate::store::{Store, StoreError};
use crate::transport::{ServedFile, ServedFiles};

/// The files of `entries` the owner's store can serve now: those whose
/// record has this blob hash and a plaintext hash (the nonce prefix's
/// source). The others are left out until the catalogue reads them.
/// Blocking.
pub fn served_files(
    store: &dyn Store,
    stick_id: &StickId,
    entries: &[Entry],
) -> Result<ServedFiles, StoreError> {
    let layout = StickLayout::load(store, stick_id)?;
    let mut recorded = store.owned_files(stick_id)?;
    recorded.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    let mut files = HashMap::with_capacity(entries.len());
    let mut unread = 0usize;
    for entry in entries {
        let Ok(index) = recorded.binary_search_by(|file| file.path.as_str().cmp(&entry.path))
        else {
            unread += 1;
            continue;
        };
        let file = &recorded[index];
        let Some(plaintext_hash) = file.plaintext_hash.filter(|_| file.blob_hash == entry.hash)
        else {
            unread += 1;
            continue;
        };
        files.insert(
            entry.hash,
            ServedFile {
                path: entry.path.as_str().into(),
                plaintext_len: entry.size,
                plaintext_hash,
            },
        );
    }
    if unread > 0 {
        tracing::debug!(unread, "files not served until the catalogue reads them");
    }
    Ok(ServedFiles { layout, files })
}

#[cfg(test)]
#[path = "served_tests.rs"]
mod tests;
