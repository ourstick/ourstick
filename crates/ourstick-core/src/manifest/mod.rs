//! The manifest is the index of a stick: every file with its path, size and hash.
//! It is the first thing a reader downloads.
//!
//! - [`build`]: creating a manifest from a folder on disk.
//! - [`reuse`]: the same, encrypting only files that changed since last time.
//! - [`encoding`]: the wire format (see `docs/protocol.md`).

pub(crate) mod build;
mod encoding;
pub mod ignore;
pub mod reuse;
mod stamp;

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use encoding::{validate_name, MANIFEST_FORMAT_VERSION};
pub use reuse::{Encrypted, KnownFiles};
pub use stamp::FileStamp;

/// The file in a drive stick's folder that says which stick it is (its
/// StickId in hex). Without it the folder is not the stick, e.g. an ejected
/// disk image's empty mount point, and is never published.
pub const DRIVE_MARKER: &str = ".ourstick-drive";

/// Index of all files in a stick.
///
/// A valid manifest has its entries strictly sorted by path (so no duplicates),
/// every path follows the rules in [`Entry::path`], and the name follows
/// [`MAX_NAME_CHARS`] and the character rules of `docs/protocol.md`. The field
/// order is the wire order (format 2).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// More files follow in a later version: the owner is still reading its
    /// folder (ADR 0007). A reader keeps waiting for a complete version.
    pub partial: bool,
    /// The stick's name as its owner chose it (ADR 0009); empty if it has
    /// none. Text from a peer: escape it before showing it.
    pub name: String,
    pub entries: Vec<Entry>,
}

/// The longest a stick's name may be, in Unicode scalar values.
pub const MAX_NAME_CHARS: usize = 60;

/// One file inside a stick.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    /// Path relative to the stick root, always with `/` separators.
    ///
    /// Never empty, never absolute, and no component is empty, `.` or `..`.
    /// Never contains `\` or NUL.
    pub path: String,
    /// Size of the file (the plaintext) in bytes.
    pub size: u64,
    /// Last modification time, in whole seconds since the UNIX epoch.
    /// Times before the epoch are clamped to 0.
    pub mtime: u64,
    /// BLAKE3 hash of the file's blob: the ciphertext, header included.
    pub hash: [u8; 32],
}

/// Errors produced while building, encoding or decoding a manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("stick root is not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot encrypt {path}: {source}")]
    Blob {
        path: PathBuf,
        #[source]
        source: crate::cipher::CipherError,
    },
    #[error("invalid path in manifest: {0:?}")]
    InvalidPath(String),
    #[error("manifest entries are not strictly sorted by path at {0:?}")]
    NotSorted(String),
    #[error("invalid stick name in manifest: {0:?}")]
    InvalidName(String),
    #[error("unsupported manifest format version {0}")]
    UnsupportedVersion(u8),
    #[error("manifest has {0} trailing bytes")]
    TrailingBytes(usize),
    #[error("manifest is empty input")]
    EmptyInput,
    #[error("malformed manifest: {0}")]
    Postcard(#[from] postcard::Error),
}

impl ManifestError {
    fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

impl Manifest {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn total_size(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_manifest_has_no_entries_and_zero_size() {
        let manifest = Manifest::empty();
        assert!(manifest.entries.is_empty());
        assert_eq!(manifest.total_size(), 0);
    }
}
