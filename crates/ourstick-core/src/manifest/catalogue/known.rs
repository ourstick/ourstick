//! What the catalogue knows of a file from its record.

use super::{CataloguedFile, Entry, FileStamp};

/// What was recorded of a file, by its path (the key).
#[derive(Debug, Clone, Copy)]
pub(super) struct Known {
    pub(super) stamp: FileStamp,
    pub(super) blob_hash: [u8; 32],
    pub(super) plaintext_hash: Option<[u8; 32]>,
}

/// A recorded file, unread, as the manifest had it.
pub(super) fn carried(path: &str, known: &Known) -> CataloguedFile {
    CataloguedFile {
        entry: Entry {
            path: path.to_owned(),
            size: known.stamp.size,
            mtime: known.stamp.mtime_ns / 1_000_000_000,
            hash: known.blob_hash,
        },
        stamp: known.stamp,
        plaintext_hash: known.plaintext_hash,
        read: false,
    }
}
