//! The manifest is the index of a stick: every file with its path, size and hash.
//! It is the first thing a reader downloads.

use serde::{Deserialize, Serialize};

/// Index of all files in a stick.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub entries: Vec<Entry>,
}

/// One file inside a stick.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    /// Path relative to the stick root, always with `/` separators.
    pub path: String,
    pub size: u64,
    /// BLAKE3 hash of the (encrypted) blob.
    pub hash: [u8; 32],
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
        let m = Manifest::empty();
        assert!(m.entries.is_empty());
        assert_eq!(m.total_size(), 0);
    }
}
