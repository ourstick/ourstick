//! What the output folder already holds, file by file, before anything is
//! fetched: a file that is already there is skipped, and a file changed there
//! since `get` wrote it (or not written by `get` at all) is never overwritten.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use super::files::component_problem;
use super::left_out::LeftOut;
use super::saved_paths::{saved_paths, OnDisk, Recorded, Rules, SavedAs};
use super::{io_error, DownloadError};
use crate::cipher::Cipher;
use crate::manifest::hashing::hash_file;
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
    /// No savable place was found for it here (every place in the way):
    /// not fetched, told.
    LeftOut,
    /// The user chose not to get it (ADR 0012): not fetched, not counted.
    NotChosen,
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
    /// The top-level names the user chose not to get that are on the stick,
    /// with their files and bytes (ADR 0012).
    pub(super) not_chosen: Vec<(String, usize, u64)>,
    /// Files found to hold the stick's content already, to record as written.
    pub(super) found: Vec<DownloadedFile>,
    /// Files `get` wrote that are still there but no longer in the stick.
    pub(super) no_longer_in_stick: usize,
}

impl FolderPlan {
    /// The bytes this plan writes, by each top-level entry of the stick
    /// and each entry inside a top-level folder they are under (a preview's
    /// per-entry "needs up to", ADR 0012 and its Amendment 1): keys of one
    /// part and of two.
    pub(super) fn bytes_by_top(&self, manifest: &Manifest) -> BTreeMap<String, u64> {
        let mut by_path: BTreeMap<String, u64> = BTreeMap::new();
        for (plan, entry) in self.files.iter().zip(&manifest.entries) {
            if matches!(plan, FilePlan::Download { .. }) {
                let mut parts = entry.path.splitn(3, '/');
                let first = parts.next().unwrap_or_default();
                let mut keys = vec![first.to_owned()];
                if let Some(second) = parts.next() {
                    keys.push(format!("{first}/{second}"));
                }
                for key in keys {
                    let bytes = by_path.entry(key).or_default();
                    *bytes = bytes.saturating_add(entry.size);
                }
            }
        }
        by_path
    }

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
    hash_file(output_dir, path, cipher)
        .ok()
        .map(|entry| entry.hash)
}

/// Looks at every file of `manifest` under `output_dir`, and where each is
/// saved there (ADR 0010). Blocking.
pub(super) fn plan_folder(
    output_dir: &Path,
    manifest: &Manifest,
    recorded: &[DownloadedFile],
    left_out: &[String],
    cipher: &dyn Cipher,
) -> Result<FolderPlan, DownloadError> {
    let rules = Rules::here(case_folded(output_dir));
    plan_folder_by(output_dir, manifest, recorded, left_out, cipher, rules)
}

/// [`plan_folder`] with the given rules (tests force Windows' anywhere).
pub(super) fn plan_folder_by(
    output_dir: &Path,
    manifest: &Manifest,
    recorded: &[DownloadedFile],
    left_out: &[String],
    cipher: &dyn Cipher,
    rules: Rules,
) -> Result<FolderPlan, DownloadError> {
    // What the user chose not to get: paths of one or two parts exactly as
    // the stick names them (ADR 0012 and its Amendment 1), decided by
    // `LeftOut` alone.
    let left_out = LeftOut::new(left_out);
    let not_chosen: std::collections::HashSet<usize> = manifest
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| left_out.covers(&entry.path).is_some())
        .map(|(index, _)| index)
        .collect();
    let looked = Looked {
        output_dir,
        folders: RefCell::default(),
    };
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
                || looked
                    .target(&file.saved_as)
                    .is_ok_and(|target| target.is_file())
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
        &not_chosen,
        |place, entry| {
            let there = looked.on_disk(place);
            // A file of this entry's content, placed before a stop, is its.
            if let (OnDisk::File, Some(index)) = (there, entry) {
                let ours = looked
                    .target(place)
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
    for (index, (entry, saved_as)) in manifest.entries.iter().zip(&places.saved).enumerate() {
        if not_chosen.contains(&index) {
            files.push(FilePlan::NotChosen);
            continue;
        }
        // No place found for it, or the last word before anything is
        // written: a name that is still not savable (it should not happen).
        if places.left_out.contains(&index)
            || saved_as
                .split('/')
                .any(|name| component_problem(name, rules.windows).is_some())
        {
            tracing::warn!("a file left out: no savable place was found for it");
            files.push(FilePlan::LeftOut);
            continue;
        }
        // A shortcut (symbolic link) at its place, or on the way to it: this
        // file is left out, the others come.
        let Ok(target) = looked.target(saved_as) else {
            tracing::warn!("a file left out: a shortcut is at its place");
            files.push(FilePlan::LeftOut);
            continue;
        };
        let metadata = match std::fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                files.push(FilePlan::Download { replaceable: None });
                continue;
            }
            // Something of the user's where a folder must go, which the
            // choice of places could not see: this file waits, the others
            // come (a failed round would stop them all, every round).
            Err(error) if error.kind() == std::io::ErrorKind::NotADirectory => {
                tracing::warn!("a file left out: a file here is in the way of its folder");
                files.push(FilePlan::LeftOut);
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
    // Told by what was left out, not by the first part of each path.
    let mut by_name: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    for index in &not_chosen {
        let entry = &manifest.entries[*index];
        let left = left_out.covers(&entry.path).unwrap_or_default();
        let counted = by_name.entry(left).or_default();
        counted.0 += 1;
        counted.1 = counted.1.saturating_add(entry.size);
    }
    Ok(FolderPlan {
        not_chosen: by_name
            .into_iter()
            .map(|(name, (files, bytes))| (name.to_owned(), files, bytes))
            .collect(),
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

mod disk;

pub(super) use disk::case_folded;
use disk::Looked;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod chosen_tests;

#[cfg(test)]
mod gauge_tests;
