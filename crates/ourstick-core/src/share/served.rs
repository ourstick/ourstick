//! What the owner's store serves for a version (ADR 0017): each file of the
//! manifest that the catalogue recorded with a plaintext hash, by blob hash,
//! with the stick's layout to find it on disk.

use std::collections::HashMap;
use std::sync::Arc;

use crate::layout::StickLayout;
use crate::manifest::Manifest;
use crate::stick_id::StickId;
use crate::store::{Store, StoreError};
use crate::transport::{ServedFile, ServedFiles};

/// The files of `manifest` the owner's store can serve now: those whose
/// record has this blob hash and a plaintext hash (the nonce prefix's
/// source), by their entry's index (ADR 0019). The others are left out
/// until the catalogue reads them. Blocking.
pub fn served_files(
    store: &dyn Store,
    stick_id: &StickId,
    manifest: Arc<Manifest>,
) -> Result<ServedFiles, StoreError> {
    let layout = StickLayout::load(store, stick_id)?;
    let mut files = HashMap::with_capacity(manifest.entries.len());
    let mut read = 0usize;
    store.each_owned_file(stick_id, &mut |file| {
        let found = manifest
            .entries
            .binary_search_by(|entry| entry.path.as_str().cmp(&file.path));
        if let (Ok(index), Some(plaintext_hash)) = (found, file.plaintext_hash) {
            let entry = &manifest.entries[index];
            if entry.hash == file.blob_hash {
                read += 1;
                files.entry(entry.hash).or_insert(ServedFile {
                    index: u32::try_from(index).unwrap_or(u32::MAX),
                    plaintext_hash,
                });
            }
        }
        std::ops::ControlFlow::Continue(())
    })?;
    let unread = manifest.entries.len().saturating_sub(read);
    if unread > 0 {
        tracing::debug!(unread, "files not served until the catalogue reads them");
    }
    Ok(ServedFiles {
        layout,
        manifest,
        files,
    })
}

#[cfg(test)]
#[path = "served_tests.rs"]
mod tests;
