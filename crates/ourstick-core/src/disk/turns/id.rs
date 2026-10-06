//! Which disk a file or folder is on.

use std::path::Path;

/// Which disk a folder is on: its filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiskId {
    /// The device id of a filesystem (Linux, macOS).
    Device(u64),
    /// A Windows volume, by its path's prefix (`C:`, also for `\\?\C:`).
    /// A `subst` letter is another prefix for a folder of another volume:
    /// OurStick reads sticks by their folders' own paths, not the letter.
    Volume(String),
    /// Not known: every such folder shares one.
    Unknown,
}

impl DiskId {
    /// The disk of the file at `path`, from its `metadata` (already taken).
    pub fn of_file(path: &Path, metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let _ = path;
            Self::Device(metadata.dev())
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Self::of(path)
        }
    }

    /// The disk `folder` is on. Blocking.
    pub fn of(folder: &Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            match std::fs::metadata(folder) {
                Ok(metadata) => Self::Device(metadata.dev()),
                Err(error) => {
                    tracing::debug!(%error, "a folder's disk is not known");
                    Self::Unknown
                }
            }
        }
        #[cfg(not(unix))]
        {
            use std::path::{Component, Prefix};
            match folder.components().next() {
                Some(Component::Prefix(prefix)) => match prefix.kind() {
                    Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                        Self::Volume(format!("{}:", (letter as char).to_ascii_uppercase()))
                    }
                    _ => Self::Volume(prefix.as_os_str().to_string_lossy().to_uppercase()),
                },
                _ => Self::Unknown,
            }
        }
    }
}
