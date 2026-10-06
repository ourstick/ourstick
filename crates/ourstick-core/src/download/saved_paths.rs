//! Where each file of a stick is saved in a received folder (ADR 0010):
//! its own path if this computer can save it there, else the smallest
//! change, the same for a folder and all its files, and the same in every
//! later version (the places recorded stay).

use std::borrow::Cow;
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
pub(crate) struct SavedPaths<'a> {
    /// Its own path (borrowed: nearly every entry), or its new place.
    pub(crate) saved: Vec<Cow<'a, str>>,
    pub(crate) renamed: Vec<SavedAs>,
    /// Recorded places this round's folding makes one: moved to a new place
    /// (stick path, new place).
    pub(crate) moved: Vec<(String, String)>,
    /// Every recorded row not kept at its place (in the stick or not), by
    /// stick path: its row is to be forgotten.
    pub(crate) displaced: Vec<String>,
    /// Entries no place was found for (every candidate in the way, e.g. a
    /// folder of the stick that is now a link elsewhere): left out, by index.
    pub(crate) left_out: Vec<usize>,
}

/// How many numbers a name tries before its entry is left out.
const MOST_NUMBERS: u32 = 1_000;

/// The names OurStick uses itself at the top of a received folder.
const OURS: [&str; 2] = [super::PARTIAL_DIR, super::receive::RECEIVED_MARKER];

/// Where each of `paths` (a manifest's, in its order) is saved in a folder
/// holding `recorded`, whose places `on_disk` tells: `on_disk(place, None)`
/// for a folder's place, `on_disk(place, Some(entry))` for a file's, where
/// the caller says `Nothing` if the file there already holds that entry's
/// content (placed, then stopped before it was recorded). Recorded places first
/// (they stay theirs), then every path that needs no change, then the rest
/// in manifest order (ADR 0010, rule 4). Entries in `not_chosen` (by index,
/// ADR 0012) take no new place and are not told as renamed.
pub(crate) fn saved_paths<'a>(
    paths: &[&'a str],
    recorded: &[Recorded<'_>],
    not_chosen: &HashSet<usize>,
    on_disk: impl Fn(&str, Option<usize>) -> OnDisk,
    rules: Rules,
) -> SavedPaths<'a> {
    // What is taken here, and where each folder of the stick is saved.
    let mut taken = Taken::default();
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
        // Its folders as other rows chose them, none of them a file now (the
        // user put a file there), and its place not made one with another
        // by this round's folding.
        let pairs = folder_pairs(row.path, row.saved_as);
        let agrees = pairs
            .iter()
            .all(|(stick, here)| taken.chosen_for(stick).is_none_or(|chosen| chosen == here));
        let in_the_way = pairs
            .iter()
            .any(|(_, here)| on_disk(here, None) == OnDisk::File);
        if !agrees || in_the_way || !taken.file_free(row.saved_as, row.path, rules) {
            tracing::warn!("a recorded place is no longer free here; chosen again");
            moved.push(row.path);
            continue;
        }
        taken.take(row.saved_as, row.path, rules);
        taken.remember_folders(row.path, row.saved_as);
        by_path.insert(row.path, row.saved_as.to_owned());
    }

    let mut saved: Vec<Option<Cow<'a, str>>> = paths
        .iter()
        .map(|path| {
            by_path
                .get(path)
                .map(|saved_as| own_or_borrowed(path, saved_as))
        })
        .collect();
    // Unchanged: every name savable as it is, its folders not moved, and
    // nothing in the way.
    // Entries the user chose not to get (ADR 0012) take no new place: their
    // recorded rows above keep theirs.
    for (index, path) in paths.iter().enumerate() {
        if not_chosen.contains(&index)
            || saved[index].is_some()
            || !unchanged(path, index, &taken, &on_disk, rules)
        {
            continue;
        }
        taken.take(path, path, rules);
        taken.remember_folders(path, path);
        saved[index] = Some(Cow::Borrowed(*path));
    }
    // The rest, top-down.
    let mut left_out = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        if not_chosen.contains(&index) {
            if saved[index].is_none() {
                saved[index] = Some(Cow::Borrowed(*path));
            }
            continue;
        }
        if saved[index].is_none() {
            match choose(path, index, &mut taken, &on_disk, rules) {
                Some(place) => saved[index] = Some(own_or_borrowed(path, &place)),
                None => {
                    tracing::warn!("a file left out: every place for it is in the way");
                    left_out.push(index);
                    saved[index] = Some(Cow::Borrowed(*path));
                }
            }
        }
    }
    let saved: Vec<Cow<'a, str>> = saved.into_iter().flatten().collect();
    let displaced = moved.iter().map(|path| (*path).to_owned()).collect();
    let index_of: HashMap<&str, usize> = paths
        .iter()
        .enumerate()
        .map(|(index, path)| (*path, index))
        .collect();
    let moved = moved
        .into_iter()
        .filter_map(|path| {
            let index = *index_of.get(path)?;
            Some((path.to_owned(), saved[index].clone().into_owned()))
        })
        .collect();
    // What was renamed, of what is saved.
    let left_set: std::collections::HashSet<usize> = left_out.iter().copied().collect();
    let (named, placed): (Vec<&str>, Vec<&str>) = paths
        .iter()
        .zip(&saved)
        .enumerate()
        .filter(|(index, _)| !left_set.contains(index) && !not_chosen.contains(index))
        .map(|(_, (path, saved))| (*path, saved.as_ref()))
        .unzip();
    SavedPaths {
        renamed: super::saved_lines::renamed(&named, &placed),
        saved,
        moved,
        displaced,
        left_out,
    }
}

/// `place`, borrowed from `path` when it is the same.
fn own_or_borrowed<'a>(path: &'a str, place: &str) -> Cow<'a, str> {
    if place == path {
        Cow::Borrowed(path)
    } else {
        Cow::Owned(place.to_owned())
    }
}

/// Whether `path` can be saved as it is: every name savable, each folder
/// either not chosen yet or chosen as itself, no file where a folder must
/// go, and its own place free.
fn unchanged(
    path: &str,
    index: usize,
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
        match taken.chosen_for(&folder) {
            Some(chosen) if chosen != folder => return false,
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
    taken: &mut Taken,
    on_disk: &impl Fn(&str, Option<usize>) -> OnDisk,
    rules: Rules,
) -> Option<String> {
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
            if let Some(chosen) = taken.chosen_for(&stick) {
                here = chosen.to_owned();
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
            if number > MOST_NUMBERS {
                return None;
            }
            candidate = join(&numbered(&base, number, !last, rules.windows));
            number += 1;
        }
        if last {
            taken.take(&candidate, path, rules);
            return Some(candidate);
        }
        taken.hold_folder(&rules.fold(&candidate), &stick);
        taken.choose_folder(&stick, &candidate);
        here = candidate;
    }
    unreachable!("a path has at least one name")
}

/// `name` made savable, and not one of OurStick's own at the top (marked
/// first, then made savable: the mark counts in its length).
fn savable(name: &str, first: bool, rules: Rules) -> String {
    let marked = if first { not_ours(name, &OURS) } else { None };
    let name = marked.as_deref().unwrap_or(name);
    safe_name(name, rules.windows).into_owned()
}

mod taken;

use taken::{folder_pairs, Taken};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod edge_tests;
