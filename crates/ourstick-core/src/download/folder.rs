//! What the output folder already holds, file by file, before anything is
//! fetched: a file that is already there is skipped, and a file changed there
//! since `get` wrote it (or not written by `get` at all) is never overwritten.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use super::files::{component_problem, target_path};
use super::saved_paths::{saved_paths, OnDisk, Recorded, Rules, SavedAs};
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
    /// Where each entry is saved, relative to the folder (ADR 0010): its
    /// path, or the smallest change of it this computer can save.
    pub(super) saved: Vec<String>,
    /// What is saved under another name, a renamed folder once.
    pub(super) renamed: Vec<SavedAs>,
    /// Recorded files to forget before anything is recorded: their file is
    /// gone and they left the stick, or they are saved at a new place.
    pub(super) forget: Vec<String>,
    /// Files found to hold the stick's content already, to record as written.
    pub(super) found: Vec<DownloadedFile>,
    /// Files `get` wrote that are still there but no longer in the stick.
    pub(super) no_longer_in_stick: usize,
}

impl FolderPlan {
    /// The bytes this plan writes into the folder: the files to download.
    pub(super) fn bytes_to_write(&self, manifest: &Manifest) -> u64 {
        self.files
            .iter()
            .zip(&manifest.entries)
            .filter(|(plan, _)| matches!(plan, FilePlan::Download { .. }))
            .map(|(_, entry)| entry.size)
            .sum()
    }
}

/// The blob hash the stick would have for this file's content, or `None` if
/// it cannot be read right now.
fn content_hash(output_dir: &Path, path: &Path, cipher: &dyn Cipher) -> Option<[u8; 32]> {
    hash_file(output_dir, path, cipher, None)
        .ok()
        .map(|(entry, _)| entry.hash)
}

/// Whether the disk of `folder` takes two names that differ only in letter
/// case for one: always on Windows; elsewhere asked of the disk itself,
/// with a name inside the folder (or inside the nearest folder above it
/// that is there, where the folder will be made) and the same name in the
/// other case: the folder's own name would be looked up on its parent's
/// disk, wrong for a disk mounted there. Where nothing tells, yes: a
/// needless " (2)" is better than two files written at one place. Reads
/// only. Blocking.
pub(super) fn case_folded(folder: &Path) -> bool {
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

/// What is at `place` under `output_dir`; a symbolic link is in the way like
/// a file.
fn on_disk(output_dir: &Path, place: &str) -> OnDisk {
    let Ok(target) = target_path(output_dir, place) else {
        return OnDisk::File;
    };
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.is_dir() => OnDisk::Folder,
        Ok(_) => OnDisk::File,
        Err(_) => OnDisk::Nothing,
    }
}

/// Looks at every file of `manifest` under `output_dir`, and where each is
/// saved there (ADR 0010). Blocking.
pub(super) fn plan_folder(
    output_dir: &Path,
    manifest: &Manifest,
    recorded: &[DownloadedFile],
    cipher: &dyn Cipher,
) -> Result<FolderPlan, DownloadError> {
    let rules = Rules::here(case_folded(output_dir));
    plan_folder_by(output_dir, manifest, recorded, cipher, rules)
}

/// [`plan_folder`] with the given rules (tests force Windows' anywhere).
pub(super) fn plan_folder_by(
    output_dir: &Path,
    manifest: &Manifest,
    recorded: &[DownloadedFile],
    cipher: &dyn Cipher,
    rules: Rules,
) -> Result<FolderPlan, DownloadError> {
    let in_stick: BTreeSet<&str> = manifest
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    // A row of a file that left the stick keeps its place while its file is
    // there; once the file is gone the row is forgotten.
    let (kept, mut forget): (Vec<&DownloadedFile>, Vec<&DownloadedFile>) =
        recorded.iter().partition(|file| {
            in_stick.contains(file.path.as_str())
                || target_path(output_dir, &file.saved_as).is_ok_and(|target| target.is_file())
        });
    let rows: Vec<Recorded<'_>> = kept
        .iter()
        .map(|file| Recorded {
            path: &file.path,
            saved_as: &file.saved_as,
        })
        .collect();
    let paths: Vec<&str> = manifest
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    // Each file here is read at most once (its content hash), by the
    // choice of places and by the plan below.
    let hashes: RefCell<HashMap<PathBuf, Option<[u8; 32]>>> = RefCell::default();
    let hash_of = |target: &Path| {
        *hashes
            .borrow_mut()
            .entry(target.to_path_buf())
            .or_insert_with(|| content_hash(output_dir, target, cipher))
    };
    let places = saved_paths(
        &paths,
        &rows,
        |place, entry| {
            let there = on_disk(output_dir, place);
            // A file of this entry's content, placed before a stop, is its.
            if let (OnDisk::File, Some(index)) = (there, entry) {
                let ours = target_path(output_dir, place)
                    .ok()
                    .and_then(|target| hash_of(&target));
                if ours == Some(manifest.entries[index].hash) {
                    return OnDisk::Nothing;
                }
            }
            there
        },
        rules,
    );
    // Displaced rows (their place made one with another's, or not this
    // path's), in the stick or not: forgotten, and recorded again at their
    // new place once there.
    let moved: BTreeSet<&str> = places.displaced.iter().map(String::as_str).collect();
    forget.extend(
        kept.iter()
            .filter(|file| moved.contains(file.path.as_str())),
    );
    let recorded: BTreeMap<&str, &DownloadedFile> = kept
        .iter()
        .filter(|file| !moved.contains(file.path.as_str()))
        .map(|file| (file.path.as_str(), *file))
        .collect();
    let mut files = Vec::with_capacity(manifest.entries.len());
    let mut found = Vec::new();
    for (entry, saved_as) in manifest.entries.iter().zip(&places.saved) {
        // The last word before anything is written: a name that is still not
        // savable (it should not happen) leaves that one file out.
        if saved_as
            .split('/')
            .any(|name| component_problem(name, rules.windows).is_some())
        {
            tracing::warn!("a file left out: no savable name was found for it");
            files.push(FilePlan::ChangedHere);
            continue;
        }
        let target = target_path(output_dir, saved_as)?;
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
            _ => match hash_of(&target) {
                Some(local) if local == entry.hash => {
                    found.push(DownloadedFile {
                        path: entry.path.clone(),
                        saved_as: saved_as.clone(),
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
    let no_longer_in_stick = recorded
        .values()
        .filter(|file| !in_stick.contains(file.path.as_str()))
        .count();
    Ok(FolderPlan {
        files,
        saved: places.saved,
        renamed: places.renamed,
        forget: forget.iter().map(|file| file.path.clone()).collect(),
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
