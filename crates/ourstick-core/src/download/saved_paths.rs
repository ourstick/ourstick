//! Where each file of a stick is saved in a received folder (ADR 0010):
//! its own path if this computer can save it there, else the smallest
//! change, the same for a folder and all its files, and the same in every
//! later version (the places recorded stay).

use std::collections::{HashMap, HashSet};

use super::saved_names::{not_ours, numbered, safe_name};

/// How this receiver's folder compares and allows names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rules {
    /// Windows' rules for names.
    pub(crate) windows: bool,
    /// The folder's disk ignores letter case (probed, see `case_folded`).
    pub(crate) fold_case: bool,
    /// The system takes two Unicode forms of a name as one (macOS).
    pub(crate) fold_normal: bool,
}

impl Rules {
    /// This system's rules, for a folder whose disk does (or does not)
    /// ignore letter case.
    pub(crate) fn here(fold_case: bool) -> Self {
        Self {
            windows: cfg!(windows),
            fold_case,
            fold_normal: cfg!(target_os = "macos"),
        }
    }

    /// `path` as this folder's disk sees it.
    fn fold(&self, path: &str) -> String {
        let cased = if self.fold_case {
            path.to_lowercase()
        } else {
            path.to_owned()
        };
        if self.fold_normal {
            icu_normalizer::ComposingNormalizerBorrowed::new_nfc()
                .normalize(&cased)
                .into_owned()
        } else {
            cased
        }
    }
}

/// What a place in the folder holds now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnDisk {
    Nothing,
    File,
    Folder,
}

/// A recorded file of this folder: its path in the stick and where it was
/// saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Recorded<'a> {
    pub(crate) path: &'a str,
    pub(crate) saved_as: &'a str,
}

/// One name saved under another: a file, or a folder for all its `files`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedAs {
    /// In the stick (`Trip?` for a folder).
    pub path: String,
    /// In the folder (`Trip_`).
    pub saved_as: String,
    /// How many of this round's files it holds (1 for a file).
    pub files: usize,
}

/// Where every entry goes, in manifest order, and what was renamed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SavedPaths {
    pub(crate) saved: Vec<String>,
    pub(crate) renamed: Vec<SavedAs>,
    /// Recorded places this round's folding makes one: moved to a new place
    /// (stick path, new place).
    pub(crate) moved: Vec<(String, String)>,
}

/// The places taken this round, as the disk sees them: files, and every
/// folder they are in.
#[derive(Default)]
struct Taken {
    files: HashSet<String>,
    folders: HashSet<String>,
}

impl Taken {
    /// Whether a file may go at `place` (folded).
    fn file_free(&self, place: &str) -> bool {
        !self.files.contains(place) && !self.folders.contains(place)
    }

    /// Whether a folder may be at `place` (folded): no file is there.
    fn folder_free(&self, place: &str) -> bool {
        !self.files.contains(place)
    }

    fn take(&mut self, place: &str) {
        let mut parent = place;
        while let Some((up, _)) = parent.rsplit_once('/') {
            self.folders.insert(up.to_owned());
            parent = up;
        }
        self.files.insert(place.to_owned());
    }
}

/// The names OurStick uses itself at the top of a received folder.
const OURS: [&str; 2] = [super::PARTIAL_DIR, super::receive::RECEIVED_MARKER];

/// Where each of `paths` (a manifest's, in its order) is saved in a folder
/// holding `recorded`, whose places `on_disk` tells: `on_disk(place, None)`
/// for a folder's place, `on_disk(place, Some(entry))` for a file's, where
/// the caller says `Nothing` if the file there already holds that entry's
/// content (placed, then stopped before it was recorded). Recorded places first
/// (they stay theirs), then every path that needs no change, then the rest
/// in manifest order (ADR 0010, rule 4).
pub(crate) fn saved_paths(
    paths: &[&str],
    recorded: &[Recorded<'_>],
    on_disk: impl Fn(&str, Option<usize>) -> OnDisk,
    rules: Rules,
) -> SavedPaths {
    let mut taken = Taken::default();
    // Stick folder → folder in the received folder, as already chosen.
    let mut folders: HashMap<String, String> = HashMap::new();
    let mut by_path: HashMap<&str, String> = HashMap::new();
    let mut moved = Vec::new();
    let mut rows: Vec<&Recorded<'_>> = recorded.iter().collect();
    rows.sort_by(|a, b| a.saved_as.cmp(b.saved_as));
    for row in rows {
        let place = rules.fold(row.saved_as);
        let parts = row.path.split('/').count();
        if parts != row.saved_as.split('/').count() || !taken.file_free(&place) {
            // Made one by this round's folding, or not a place of this path:
            // chosen again below.
            if parts != row.saved_as.split('/').count() {
                tracing::warn!("a recorded place does not match its path; chosen again");
            }
            moved.push(row.path);
            continue;
        }
        taken.take(&place);
        remember_folders(&mut folders, row.path, row.saved_as);
        by_path.insert(row.path, row.saved_as.to_owned());
    }

    let mut saved: Vec<Option<String>> = paths
        .iter()
        .map(|path| by_path.get(path).cloned())
        .collect();
    // Unchanged: every name savable as it is, its folders not moved, and
    // nothing in the way.
    for (index, path) in paths.iter().enumerate() {
        if saved[index].is_some() || !unchanged(path, &folders, &taken, &on_disk, rules) {
            continue;
        }
        taken.take(&rules.fold(path));
        remember_folders(&mut folders, path, path);
        saved[index] = Some((*path).to_owned());
    }
    // The rest, top-down.
    for (index, path) in paths.iter().enumerate() {
        if saved[index].is_none() {
            let place = choose(path, index, &mut folders, &mut taken, &on_disk, rules);
            saved[index] = Some(place);
        }
    }
    let saved: Vec<String> = saved.into_iter().flatten().collect();
    let moved = moved
        .into_iter()
        .filter_map(|path| {
            let index = paths.iter().position(|p| *p == path)?;
            Some((path.to_owned(), saved[index].clone()))
        })
        .collect();
    SavedPaths {
        renamed: renamed(paths, &saved),
        saved,
        moved,
    }
}

/// Records the folders of `path` as saved at those of `saved_as`.
fn remember_folders(folders: &mut HashMap<String, String>, path: &str, saved_as: &str) {
    let mut stick = String::new();
    let mut here = String::new();
    let parts: Vec<(&str, &str)> = path.split('/').zip(saved_as.split('/')).collect();
    for (stick_name, saved_name) in &parts[..parts.len().saturating_sub(1)] {
        if !stick.is_empty() {
            stick.push('/');
            here.push('/');
        }
        stick.push_str(stick_name);
        here.push_str(saved_name);
        folders.entry(stick.clone()).or_insert_with(|| here.clone());
    }
}

/// Whether `path` can be saved as it is: every name savable, each folder
/// either not chosen yet or chosen as itself, no file where a folder must
/// go, and its own place free.
fn unchanged(
    path: &str,
    folders: &HashMap<String, String>,
    taken: &Taken,
    on_disk: &impl Fn(&str, Option<usize>) -> OnDisk,
    rules: Rules,
) -> bool {
    let names: Vec<&str> = path.split('/').collect();
    if not_ours(names[0], &OURS).is_some() {
        return false;
    }
    if names
        .iter()
        .any(|name| safe_name(name, rules.windows) != *name)
    {
        return false;
    }
    let mut folder = String::new();
    for name in &names[..names.len() - 1] {
        if !folder.is_empty() {
            folder.push('/');
        }
        folder.push_str(name);
        match folders.get(&folder) {
            Some(chosen) if chosen != &folder => return false,
            Some(_) => {}
            None => {
                if !taken.folder_free(&rules.fold(&folder))
                    || on_disk(&folder, None) == OnDisk::File
                {
                    return false;
                }
            }
        }
    }
    taken.file_free(&rules.fold(path))
}

/// The place for `path`: each folder as chosen before, or chosen now
/// (savable, numbered past files in the way); then the file's own name,
/// numbered past every place taken or on the disk.
fn choose(
    path: &str,
    entry: usize,
    folders: &mut HashMap<String, String>,
    taken: &mut Taken,
    on_disk: &impl Fn(&str, Option<usize>) -> OnDisk,
    rules: Rules,
) -> String {
    let names: Vec<&str> = path.split('/').collect();
    let mut stick = String::new();
    let mut here = String::new();
    for (depth, name) in names.iter().enumerate() {
        let last = depth == names.len() - 1;
        if !stick.is_empty() {
            stick.push('/');
        }
        stick.push_str(name);
        if !last {
            if let Some(chosen) = folders.get(&stick) {
                here = chosen.clone();
                continue;
            }
        }
        let base = savable(name, depth == 0, rules);
        let join = |name: &str| {
            if here.is_empty() {
                name.to_owned()
            } else {
                format!("{here}/{name}")
            }
        };
        let free = |candidate: &str| {
            let folded = rules.fold(candidate);
            if last {
                taken.file_free(&folded) && on_disk(candidate, Some(entry)) == OnDisk::Nothing
            } else {
                taken.folder_free(&folded) && on_disk(candidate, None) != OnDisk::File
            }
        };
        let mut candidate = join(&base);
        let mut number = 2;
        while !free(&candidate) {
            candidate = join(&numbered(&base, number, rules.windows));
            number += 1;
        }
        if last {
            taken.take(&rules.fold(&candidate));
            return candidate;
        }
        folders.insert(stick.clone(), candidate.clone());
        here = candidate;
    }
    unreachable!("a path has at least one name")
}

/// `name` made savable, and not one of OurStick's own at the top.
fn savable(name: &str, first: bool, rules: Rules) -> String {
    let safe = safe_name(name, rules.windows).into_owned();
    if first {
        if let Some(marked) = not_ours(&safe, &OURS) {
            return marked;
        }
    }
    safe
}

/// What was saved under another name, a renamed folder once for its files.
fn renamed(paths: &[&str], saved: &[String]) -> Vec<SavedAs> {
    let mut lines: Vec<SavedAs> = Vec::new();
    let mut index_of: HashMap<(String, String), usize> = HashMap::new();
    for (path, saved_as) in paths.iter().zip(saved) {
        if *path == saved_as {
            continue;
        }
        // The first name that differs: a folder for all its files, or the
        // file itself.
        let stick: Vec<&str> = path.split('/').collect();
        let here: Vec<&str> = saved_as.split('/').collect();
        let differs = stick
            .iter()
            .zip(&here)
            .position(|(a, b)| a != b)
            .unwrap_or(stick.len() - 1);
        let key = (stick[..=differs].join("/"), here[..=differs].join("/"));
        match index_of.get(&key) {
            Some(&index) => lines[index].files += 1,
            None => {
                index_of.insert(key.clone(), lines.len());
                lines.push(SavedAs {
                    path: key.0,
                    saved_as: key.1,
                    files: 1,
                });
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests;
