//! The places a round has taken in a received folder, as its disk sees
//! them (ADR 0010).

use std::collections::{HashMap, HashSet};

use super::Rules;

/// The places taken this round, as the disk sees them (folded): files, and
/// every folder they are in, with the stick's folder each one holds (two
/// folders of the stick are never one folder here).
#[derive(Default)]
pub(super) struct Taken {
    pub(super) files: HashSet<String>,
    pub(super) folders: HashMap<String, String>,
}

impl Taken {
    /// Whether a folder may be at `place` (folded) for the stick's `folder`:
    /// no file there, and no other stick folder.
    pub(super) fn folder_free(&self, place: &str, folder: &str) -> bool {
        !self.files.contains(place)
            && self
                .folders
                .get(place)
                .is_none_or(|holder| holder == folder)
    }

    /// Whether the file `path` of the stick may be saved at `saved_as`: its
    /// place free, and each of its folders free for the stick's folder.
    pub(super) fn file_free(&self, saved_as: &str, path: &str, rules: Rules) -> bool {
        let place = rules.fold(saved_as);
        !self.files.contains(&place)
            && !self.folders.contains_key(&place)
            && folder_pairs(path, saved_as)
                .iter()
                .all(|(stick, here)| self.folder_free(&rules.fold(here), stick))
    }

    pub(super) fn take(&mut self, saved_as: &str, path: &str, rules: Rules) {
        for (stick, here) in folder_pairs(path, saved_as) {
            self.folders.entry(rules.fold(&here)).or_insert(stick);
        }
        self.files.insert(rules.fold(saved_as));
    }
}

/// Each folder of `path` with the folder of `saved_as` it is saved as,
/// outermost first (the two have as many names).
pub(super) fn folder_pairs(path: &str, saved_as: &str) -> Vec<(String, String)> {
    let stick: Vec<&str> = path.split('/').collect();
    let here: Vec<&str> = saved_as.split('/').collect();
    (1..stick.len().min(here.len()))
        .map(|depth| (stick[..depth].join("/"), here[..depth].join("/")))
        .collect()
}
