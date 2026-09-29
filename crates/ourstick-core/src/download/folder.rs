//! What the output folder already holds, file by file, before anything is
//! fetched: a file that is already there is skipped, and a file changed there
//! since `get` wrote it (or not written by `get` at all) is never overwritten.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::files::target_path;
use super::{io_error, DownloadError};
use crate::cipher::Cipher;
use crate::manifest::build::hash_file;
use crate::manifest::{FileStamp, Manifest};
use crate::store::DownloadedFile;

/// What to do with one entry of the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FilePlan {
    /// Fetch and write it. `replaceable`: the stamp the file on disk must
    /// still have to be replaced; `None` if there must be no file.
    Download { replaceable: Option<FileStamp> },
    /// The folder already has this content.
    AlreadyHere,
    /// The folder has something else, changed there or not written by `get`:
    /// left as it is.
    ChangedHere,
}

/// The plan for every entry of a manifest, in manifest order.
#[derive(Debug)]
pub(super) struct FolderPlan {
    pub(super) files: Vec<FilePlan>,
    /// Files found to hold the stick's content already, to record as written.
    pub(super) found: Vec<DownloadedFile>,
    /// Files `get` wrote that are still there but no longer in the stick.
    pub(super) no_longer_in_stick: usize,
}

/// The blob hash the stick would have for this file's content, or `None` if
/// it cannot be read right now.
fn content_hash(output_dir: &Path, path: &Path, cipher: &dyn Cipher) -> Option<[u8; 32]> {
    hash_file(output_dir, path, cipher, None)
        .ok()
        .map(|(entry, _)| entry.hash)
}

/// Looks at every file of `manifest` under `output_dir`. Blocking.
pub(super) fn plan_folder(
    output_dir: &Path,
    manifest: &Manifest,
    recorded: &[DownloadedFile],
    cipher: &dyn Cipher,
) -> Result<FolderPlan, DownloadError> {
    let recorded: BTreeMap<&str, &DownloadedFile> = recorded
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    let mut files = Vec::with_capacity(manifest.entries.len());
    let mut found = Vec::new();
    for entry in &manifest.entries {
        let target = target_path(output_dir, &entry.path)?;
        let metadata = match std::fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                files.push(FilePlan::Download { replaceable: None });
                continue;
            }
            Err(error) => return Err(io_error(&target)(error)),
        };
        if !metadata.is_file() {
            files.push(FilePlan::ChangedHere);
            continue;
        }
        let stamp = FileStamp::of(&metadata);
        let row = recorded.get(entry.path.as_str());
        let plan = match row {
            // As `get` left it: the hash says whether the stick changed it.
            Some(row) if row.stamp == stamp => {
                if row.blob_hash == entry.hash {
                    FilePlan::AlreadyHere
                } else {
                    FilePlan::Download {
                        replaceable: Some(stamp),
                    }
                }
            }
            // Touched here, or never written by `get`: the content decides.
            _ => match content_hash(output_dir, &target, cipher) {
                Some(local) if local == entry.hash => {
                    found.push(DownloadedFile {
                        path: entry.path.clone(),
                        stamp,
                        blob_hash: entry.hash,
                    });
                    FilePlan::AlreadyHere
                }
                // Only its times changed; the content is what `get` wrote.
                Some(local) if row.is_some_and(|row| row.blob_hash == local) => {
                    FilePlan::Download {
                        replaceable: Some(stamp),
                    }
                }
                _ => FilePlan::ChangedHere,
            },
        };
        files.push(plan);
    }
    let in_stick: BTreeSet<&str> = manifest
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    let no_longer_in_stick = recorded
        .keys()
        .filter(|path| !in_stick.contains(*path))
        .filter(|path| target_path(output_dir, path).is_ok_and(|target| target.is_file()))
        .count();
    Ok(FolderPlan {
        files,
        found,
        no_longer_in_stick,
    })
}

/// Whether the file at `target` may be replaced now: it must be absent, or
/// still have the stamp it had when the download was planned.
pub(super) fn may_replace(target: &Path, replaceable: Option<FileStamp>) -> bool {
    match std::fs::symlink_metadata(target) {
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        Ok(metadata) => metadata.is_file() && Some(FileStamp::of(&metadata)) == replaceable,
    }
}

#[cfg(test)]
mod tests;
