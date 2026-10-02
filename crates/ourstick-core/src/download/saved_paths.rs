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
    /// Every recorded row not kept at its place (in the stick or not), by
    /// stick path: its row is to be forgotten.
    pub(crate) displaced: Vec<String>,
}

/// The places taken this round, as the disk sees them (folded): files, and
/// every folder they are in, with the stick's folder each one holds (two
/// folders of the stick are never one folder here).
#[derive(Default)]
struct Taken {
    files: HashSet<String>,
    folders: HashMap<String, String>,
}

impl Taken {
    /// Whether a folder may be at `place` (folded) for the stick's `folder`:
    /// no file there, and no other stick folder.
    fn folder_free(&self, place: &str, folder: &str) -> bool {
        !self.files.contains(place)
            && self
                .folders
                .get(place)
                .is_none_or(|holder| holder == folder)
    }

    /// Whether the file `path` of the stick may be saved at `saved_as`: its
    /// place free, and each of its folders free for the stick's folder.
    fn file_free(&self, saved_as: &str, path: &str, rules: Rules) -> bool {
        let place = rules.fold(saved_as);
        !self.files.contains(&place)
            && !self.folders.contains_key(&place)
            && folder_pairs(path, saved_as)
                .iter()
                .all(|(stick, here)| self.folder_free(&rules.fold(here), stick))
    }

    fn take(&mut self, saved_as: &str, path: &str, rules: Rules) {
        for (stick, here) in folder_pairs(path, saved_as) {
            self.folders.entry(rules.fold(&here)).or_insert(stick);
        }
        self.files.insert(rules.fold(saved_as));
    }
}

/// Each folder of `path` with the folder of `saved_as` it is saved as,
/// outermost first (the two have as many names).
fn folder_pairs(path: &str, saved_as: &str) -> Vec<(String, String)> {
    let stick: Vec<&str> = path.split('/').collect();
    let here: Vec<&str> = saved_as.split('/').collect();
    (1..stick.len().min(here.len()))
        .map(|depth| (stick[..depth].join("/"), here[..depth].join("/")))
        .collect()
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
        if row.path.split('/').count() != row.saved_as.split('/').count() {
            tracing::warn!("a recorded place does not match its path; chosen again");
            moved.push(row.path);
            continue;
        }
        // Its folders as other rows chose them, and its place not made one
        // with another by this round's folding.
        let agrees = folder_pairs(row.path, row.saved_as)
            .iter()
            .all(|(stick, here)| folders.get(stick).is_none_or(|chosen| chosen == here));
        if !agrees || !taken.file_free(row.saved_as, row.path, rules) {
            moved.push(row.path);
            continue;
        }
        taken.take(row.saved_as, row.path, rules);
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
        if saved[index].is_some() || !unchanged(path, index, &folders, &taken, &on_disk, rules) {
            continue;
        }
        taken.take(path, path, rules);
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
    let displaced = moved.iter().map(|path| (*path).to_owned()).collect();
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
        displaced,
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
    index: usize,
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
                if !taken.folder_free(&rules.fold(&folder), &folder)
                    || on_disk(&folder, None) == OnDisk::File
                {
                    return false;
                }
            }
        }
    }
    // A folder where the file must go is in the way, as for a renamed one.
    taken.file_free(path, path, rules) && on_disk(path, Some(index)) != OnDisk::Folder
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
            if last {
                taken.file_free(candidate, path, rules)
                    && on_disk(candidate, Some(entry)) == OnDisk::Nothing
            } else {
                taken.folder_free(&rules.fold(candidate), &stick)
                    && on_disk(candidate, None) != OnDisk::File
            }
        };
        let mut candidate = join(&base);
        let mut number = 2;
        while !free(&candidate) {
            candidate = join(&numbered(&base, number, !last, rules.windows));
            number += 1;
        }
        if last {
            taken.take(&candidate, path, rules);
            return candidate;
        }
        taken
            .folders
            .entry(rules.fold(&candidate))
            .or_insert_with(|| stick.clone());
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
