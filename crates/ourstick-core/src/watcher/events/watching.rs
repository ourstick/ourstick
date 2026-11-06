//! What the system watches for a [`NotifyEvents`](super::NotifyEvents):
//! its roots and the folders watched for them. Changed on a blocking thread
//! (see `NotifyEvents::watching`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use notify::{RecursiveMode, Watcher as _};

use super::{lock, watched_for, Roots, Target, WatchError};

pub(super) struct Watching {
    // Kept alive: dropping it stops the watch.
    pub(super) watcher: notify::RecommendedWatcher,
    /// Each root, and the folder the system watches for it ([`watched_for`]).
    pub(super) watched: BTreeMap<PathBuf, PathBuf>,
    /// The folders the system watches.
    pub(super) targets: BTreeMap<PathBuf, Target>,
    /// The same roots, for the system's callback.
    pub(super) roots: Arc<Mutex<Roots>>,
}

impl Watching {
    /// [`FolderEvents::watch_roots`](super::FolderEvents::watch_roots), as
    /// long as the system takes.
    pub(super) fn watch_roots(&mut self, roots: &[PathBuf]) -> Vec<WatchError> {
        let lost = std::mem::take(&mut lock(&self.roots).lost);
        for root in lost {
            // Watched again below, as a new root, if it is still wanted.
            self.remove_root(&root);
        }
        let wanted: BTreeSet<PathBuf> = roots.iter().cloned().collect();
        let gone: Vec<PathBuf> = self
            .watched
            .keys()
            .filter(|root| !wanted.contains(*root))
            .cloned()
            .collect();
        for root in gone {
            self.remove_root(&root);
        }
        let new: Vec<PathBuf> = wanted
            .into_iter()
            .filter(|root| !self.watched.contains_key(root))
            .collect();
        new.into_iter()
            .filter_map(|root| self.add_root(&root).err())
            .collect()
    }

    /// Watches `root` from now on, as [`watched_for`] says.
    pub(super) fn add_root(&mut self, root: &Path) -> Result<(), WatchError> {
        let error = |reason: notify::Error| WatchError {
            path: root.display().to_string(),
            reason: reason.to_string(),
        };
        let (folder, recursive) = watched_for(root);
        let (target, through_folder) = match self.watch_target(&folder, recursive) {
            Ok(()) => (folder, !recursive),
            // A file in a folder that cannot be listed (`0711`): the file
            // itself, as before; then only saves that write in place are seen.
            Err(_) if !recursive => {
                self.watch_target(root, false).map_err(error)?;
                (root.to_path_buf(), false)
            }
            Err(reason) => return Err(error(reason)),
        };
        let mut roots = self
            .roots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        roots.all.insert(root.to_path_buf());
        if through_folder {
            roots.files.insert(root.to_path_buf());
        }
        drop(roots);
        self.watched.insert(root.to_path_buf(), target);
        Ok(())
    }

    /// The system watches `target` (all under it too if `recursive`), for
    /// one root more. A folder watched already is watched again only to
    /// take in all under it.
    fn watch_target(&mut self, target: &Path, recursive: bool) -> Result<(), notify::Error> {
        let known = self.targets.get(target).map(|known| known.recursive);
        if known.is_none() || (recursive && known == Some(false)) {
            if known.is_some() {
                let _ = self.watcher.unwatch(target);
            }
            let mode = if recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            self.watcher.watch(target, mode)?;
        }
        let entry = self.targets.entry(target.to_path_buf()).or_insert(Target {
            roots: 0,
            recursive,
        });
        entry.roots += 1;
        entry.recursive |= recursive;
        Ok(())
    }

    /// Stops watching `root`, and its folder once no root needs it.
    fn remove_root(&mut self, root: &Path) {
        let Some(target) = self.watched.remove(root) else {
            return;
        };
        let mut roots = self
            .roots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        roots.all.remove(root);
        roots.files.remove(root);
        drop(roots);
        let unused = self.targets.get_mut(&target).is_some_and(|entry| {
            entry.roots = entry.roots.saturating_sub(1);
            entry.roots == 0
        });
        if unused {
            self.targets.remove(&target);
            // A folder that went is no longer watched by the system anyway.
            let _ = self.watcher.unwatch(&target);
        }
    }
}
