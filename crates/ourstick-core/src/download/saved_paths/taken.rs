//! The places a round has taken in a received folder, as its disk sees
//! them, and where each folder of the stick is saved (ADR 0010).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::Rules;

/// Every folder path a round names (in the stick, in the received folder,
/// folded for its disk), each held once: they are nearly always the same
/// string, and a stick of 100 000 files has as many folders (ADR 0018).
#[derive(Default)]
pub(super) struct Paths {
    ids: HashMap<Rc<str>, u32>,
    paths: Vec<Rc<str>>,
}

impl Paths {
    /// The id of `path`, made if it has none.
    fn id(&mut self, path: &str) -> u32 {
        if let Some(id) = self.ids.get(path) {
            return *id;
        }
        let id = u32::try_from(self.paths.len()).expect("fewer than 2^32 folders");
        let path: Rc<str> = path.into();
        self.paths.push(path.clone());
        self.ids.insert(path, id);
        id
    }

    fn get(&self, path: &str) -> Option<u32> {
        self.ids.get(path).copied()
    }

    fn path(&self, id: u32) -> &str {
        &self.paths[id as usize]
    }
}

/// The places taken this round, as the disk sees them (folded): files, and
/// every folder they are in, with the stick's folder each one holds (two
/// folders of the stick are never one folder here); and the folder each
/// folder of the stick is saved as.
#[derive(Default)]
pub(super) struct Taken {
    paths: Paths,
    files: HashSet<Box<str>>,
    /// Folded place → the stick's folder it holds.
    folders: HashMap<u32, u32>,
    /// The stick's folder → the folder it is saved as here.
    chosen: HashMap<u32, u32>,
}

impl Taken {
    /// Whether a folder may be at `place` (folded) for the stick's `folder`:
    /// no file there, and no other stick folder.
    pub(super) fn folder_free(&self, place: &str, folder: &str) -> bool {
        !self.files.contains(place)
            && self
                .paths
                .get(place)
                .and_then(|place| self.folders.get(&place))
                .is_none_or(|holder| self.paths.path(*holder) == folder)
    }

    /// Whether the file `path` of the stick may be saved at `saved_as`: its
    /// place free, and each of its folders free for the stick's folder.
    pub(super) fn file_free(&self, saved_as: &str, path: &str, rules: Rules) -> bool {
        let place = rules.fold(saved_as);
        !self.files.contains(place.as_str())
            && self
                .paths
                .get(&place)
                .is_none_or(|place| !self.folders.contains_key(&place))
            && folder_pairs(path, saved_as)
                .iter()
                .all(|(stick, here)| self.folder_free(&rules.fold(here), stick))
    }

    /// The file `path` of the stick is saved at `saved_as`: its place and
    /// its folders' are taken.
    pub(super) fn take(&mut self, saved_as: &str, path: &str, rules: Rules) {
        for (stick, here) in folder_pairs(path, saved_as) {
            self.hold_folder(&rules.fold(&here), &stick);
        }
        self.files.insert(rules.fold(saved_as).into_boxed_str());
    }

    /// The folder at `place` (folded) holds the stick's `folder`, unless it
    /// holds one already.
    pub(super) fn hold_folder(&mut self, place: &str, folder: &str) {
        let place = self.paths.id(place);
        let folder = self.paths.id(folder);
        self.folders.entry(place).or_insert(folder);
    }

    /// Where the stick's `folder` is saved, once chosen.
    pub(super) fn chosen_for(&self, folder: &str) -> Option<&str> {
        let folder = self.paths.get(folder)?;
        self.chosen.get(&folder).map(|here| self.paths.path(*here))
    }

    /// The stick's `folder` is saved as `here` from now on.
    pub(super) fn choose_folder(&mut self, folder: &str, here: &str) {
        let folder = self.paths.id(folder);
        let here = self.paths.id(here);
        self.chosen.insert(folder, here);
    }

    /// Records the folders of `path` as saved at those of `saved_as`, each
    /// unless chosen already.
    pub(super) fn remember_folders(&mut self, path: &str, saved_as: &str) {
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
            let folder = self.paths.id(&stick);
            if !self.chosen.contains_key(&folder) {
                let here = self.paths.id(&here);
                self.chosen.insert(folder, here);
            }
        }
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
