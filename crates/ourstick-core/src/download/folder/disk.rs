//! What a plan asks of the received folder's disk: whether it folds letter
//! case, and what is at a place (each folder asked once).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::super::saved_paths::OnDisk;
use super::super::DownloadError;

/// Whether the disk of `folder` takes two names that differ only in letter
/// case for one: always on Windows; elsewhere asked of the disk itself,
/// with a name inside the folder (or inside the nearest folder above it
/// that is there, where the folder will be made) and the same name in the
/// other case: the folder's own name would be looked up on its parent's
/// disk, wrong for a disk mounted there. Where nothing tells, yes: a
/// needless " (2)" is better than two files written at one place. Reads
/// only. Blocking.
pub(in crate::download) fn case_folded(folder: &Path) -> bool {
    if cfg!(windows) {
        return true;
    }
    let Some(inside) = folder.ancestors().find(|place| place.is_dir()) else {
        return true;
    };
    let Ok(children) = std::fs::read_dir(inside) else {
        return true;
    };
    for child in children.flatten().take(64) {
        let name = child.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        // ASCII letters only: their other case is one letter, always.
        if !name.chars().any(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let swapped: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_lowercase() {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect();
        let Ok(here) = std::fs::symlink_metadata(child.path()) else {
            continue;
        };
        return match std::fs::symlink_metadata(inside.join(&swapped)) {
            Ok(there) => same_file(&here, &there),
            Err(_) => false,
        };
    }
    true
}

/// Whether two metadata are of one file.
#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
}

/// What a plan learns of the folder, each place asked once: whether it is
/// a symbolic link, and what is there.
pub(super) struct Looked<'a> {
    pub(super) output_dir: &'a Path,
    pub(super) links: RefCell<HashMap<PathBuf, bool>>,
}

impl Looked<'_> {
    /// [`target_path`], with each folder asked once.
    pub(super) fn target(&self, place: &str) -> Result<PathBuf, DownloadError> {
        super::super::files::target_path_by(self.output_dir, place, &|path| {
            *self
                .links
                .borrow_mut()
                .entry(path.to_path_buf())
                .or_insert_with(|| {
                    std::fs::symlink_metadata(path)
                        .is_ok_and(|metadata| metadata.file_type().is_symlink())
                })
        })
    }

    /// What is at `place`; a symbolic link is in the way like a file.
    pub(super) fn on_disk(&self, place: &str) -> OnDisk {
        let Ok(target) = self.target(place) else {
            return OnDisk::File;
        };
        match std::fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.is_dir() => OnDisk::Folder,
            Ok(_) => OnDisk::File,
            Err(_) => OnDisk::Nothing,
        }
    }
}
