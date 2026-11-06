//! Signs that a folder may have changed: from the operating system through
//! `notify`, or on a timer when watching is not possible.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::event::{AccessKind, AccessMode, ModifyKind};
use notify::{Event, EventKind, RecursiveMode, Watcher as _};
use tokio::sync::mpsc;

/// Tells when a folder may have changed. A sign may be spurious; the caller
/// checks what actually changed.
pub trait FolderEvents: Send {
    /// Waits for the next sign of change. `None` once there will be no more.
    fn changed(&mut self) -> impl Future<Output = Option<()>> + Send;

    /// Watches exactly `roots` from now on (a stick's own folder and its
    /// linked entries, ADR 0008), as far as it can; returns those it cannot
    /// watch, whose changes are then seen only by looking again now and
    /// then. Sources that watch nothing in particular accept anything.
    fn watch_roots(&mut self, roots: &[PathBuf]) -> Vec<WatchError> {
        let _ = roots;
        Vec::new()
    }
}

/// Gives a source a sign of change from outside, e.g. when the app links a
/// folder to the stick. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Nudge(mpsc::Sender<()>);

impl Nudge {
    pub fn now(&self) {
        signal(&self.0);
    }
}

/// Signs of change wait in a channel of one: a sign that waits already
/// says "something changed", so a flood of events while a pass runs (a
/// busy log in the folder) costs nothing.
fn signs() -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
    mpsc::channel(1)
}

/// One sign more, unless one waits already. Nobody to tell once watching
/// has stopped.
fn signal(sender: &mpsc::Sender<()>) {
    let _ = sender.try_send(());
}

/// Why a folder cannot be watched. The message is meant for users.
#[derive(Debug, thiserror::Error)]
#[error("cannot watch {path} for changes: {reason}")]
pub struct WatchError {
    pub path: String,
    pub reason: String,
}

/// Changes reported by the operating system (inotify, FSEvents, …).
pub struct NotifyEvents {
    // Kept alive: dropping it stops the watch.
    watcher: notify::RecommendedWatcher,
    sender: mpsc::Sender<()>,
    receiver: mpsc::Receiver<()>,
    /// Each root, and the folder the system watches for it ([`watched_for`]).
    watched: BTreeMap<PathBuf, PathBuf>,
    /// The folders the system watches.
    targets: BTreeMap<PathBuf, Target>,
    /// The same roots, for the system's callback.
    roots: Arc<Mutex<Roots>>,
}

/// A folder the system watches: for how many roots, and whether all under
/// it too.
struct Target {
    roots: usize,
    recursive: bool,
}

/// The roots, as the system's callback sees them.
#[derive(Default)]
struct Roots {
    all: BTreeSet<PathBuf>,
    /// The roots that are a file sent alone, watched through their folder.
    files: BTreeSet<PathBuf>,
    /// Folder roots renamed or removed since the last look: notify drops
    /// their watch, even when they come straight back (on Linux, when their
    /// parent folder is watched for a file beside them), so they are
    /// watched again at the next look.
    lost: BTreeSet<PathBuf>,
}

/// What the system watches for `root`, and whether all under it: a folder
/// with all under it; a file sent alone through its folder alone. Most
/// editors save by writing a new file and renaming it over the old one: a
/// watch of the file itself would go with the old one, and no later save
/// would be seen.
fn watched_for(root: &Path) -> (PathBuf, bool) {
    let file = std::fs::metadata(root).is_ok_and(|facts| facts.is_file());
    match root
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        Some(folder) if file => (folder.to_path_buf(), false),
        _ => (root.to_path_buf(), true),
    }
}

/// Whether `path` is only beside a file sent alone: in that file's folder,
/// which is watched for it, but neither the file nor under any root.
/// Another file changing there (the user's other documents) is none of the
/// stick's.
fn beside_a_file_root(path: &Path, roots: &Roots) -> bool {
    let folder = path.parent();
    let name = path.file_name().map(|name| name.to_string_lossy());
    // The file named in another case or Unicode form (a disk that ignores
    // case) is the file itself: an extra pass at worst.
    let named_so = |file: &PathBuf| match (file.file_name(), &name) {
        (Some(file), Some(name)) => crate::stick_names::same_name(&file.to_string_lossy(), name),
        _ => false,
    };
    let files_here: Vec<&PathBuf> = roots
        .files
        .iter()
        .filter(|file| file.parent() == folder)
        .collect();
    !files_here.is_empty()
        && !files_here.into_iter().any(named_so)
        && !roots.all.iter().any(|root| path.starts_with(root))
}

/// Whether `path` is reached through a link below one of `roots`. A stick
/// never takes what is behind a link (the walk does not follow one), but
/// notify's recursive watch on Linux does: a link to a busy folder outside
/// the stick (Downloads, /tmp) would start a pass at every write there.
fn through_a_link(path: &Path, roots: &BTreeSet<PathBuf>) -> bool {
    let Some(root) = roots.iter().find(|root| path.starts_with(root)) else {
        return false;
    };
    // Only the folders inside the root: a root that is a file has none, and
    // a link above a root (a home folder reached through one) is the user's.
    path.ancestors()
        .skip(1)
        .take_while(|folder| folder.starts_with(root) && *folder != root.as_path())
        .any(|folder| {
            std::fs::symlink_metadata(folder).is_ok_and(|facts| facts.file_type().is_symlink())
        })
}

/// Whether an event may mean the folder's content changed. Opening and
/// reading are not changes, and `share` reads every file it encrypts, so
/// counting them would publish forever. A backend error (e.g. a lost event
/// queue) counts: something may have changed unseen.
fn is_change(event: &notify::Result<Event>) -> bool {
    match event {
        Err(_) => true,
        Ok(event) => match event.kind {
            EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
            EventKind::Access(_) => false,
            _ => true,
        },
    }
}

impl NotifyEvents {
    /// Starts watching `root` and everything under it.
    pub fn start(root: &Path) -> Result<Self, WatchError> {
        let (sender, receiver) = signs();
        let from_system = sender.clone();
        let roots = Arc::new(Mutex::new(Roots::default()));
        let seen_roots = roots.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            // Reads (the pass's own among them) are dropped before any
            // look at the disk for links.
            if !is_change(&event) {
                return;
            }
            let none_of_ours = event.as_ref().is_ok_and(|event| {
                let mut roots = seen_roots
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let moved = matches!(
                    event.kind,
                    EventKind::Modify(ModifyKind::Name(_)) | EventKind::Remove(_)
                );
                if moved {
                    let lost: Vec<PathBuf> = event
                        .paths
                        .iter()
                        .filter(|path| roots.all.contains(*path) && !roots.files.contains(*path))
                        .cloned()
                        .collect();
                    roots.lost.extend(lost);
                }
                !event.paths.is_empty()
                    && event.paths.iter().all(|path| {
                        through_a_link(path, &roots.all) || beside_a_file_root(path, &roots)
                    })
            });
            if !none_of_ours {
                signal(&from_system);
            }
        })
        .map_err(|reason| WatchError {
            path: root.display().to_string(),
            reason: reason.to_string(),
        })?;
        let mut events = Self {
            watcher,
            sender,
            receiver,
            watched: BTreeMap::new(),
            targets: BTreeMap::new(),
            roots,
        };
        events.add_root(root)?;
        Ok(events)
    }

    /// A way to give this source a sign of change from outside.
    pub fn nudge(&self) -> Nudge {
        Nudge(self.sender.clone())
    }

    /// Watches `root` from now on, as [`watched_for`] says.
    fn add_root(&mut self, root: &Path) -> Result<(), WatchError> {
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

impl FolderEvents for NotifyEvents {
    async fn changed(&mut self) -> Option<()> {
        self.receiver.recv().await
    }

    fn watch_roots(&mut self, roots: &[PathBuf]) -> Vec<WatchError> {
        let lost = std::mem::take(
            &mut self
                .roots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .lost,
        );
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
}

/// A sign of change every `interval`, for when the folder cannot be watched.
pub struct PeriodicEvents {
    interval: tokio::time::Interval,
    sender: mpsc::Sender<()>,
    receiver: mpsc::Receiver<()>,
}

impl PeriodicEvents {
    pub fn new(interval: Duration) -> Self {
        let start = tokio::time::Instant::now() + interval;
        let (sender, receiver) = signs();
        Self {
            interval: tokio::time::interval_at(start, interval),
            sender,
            receiver,
        }
    }

    /// A way to give this source a sign of change from outside.
    pub fn nudge(&self) -> Nudge {
        Nudge(self.sender.clone())
    }
}

impl FolderEvents for PeriodicEvents {
    async fn changed(&mut self) -> Option<()> {
        tokio::select! {
            _ = self.interval.tick() => {}
            _ = self.receiver.recv() => {}
        }
        Some(())
    }
}

#[cfg(test)]
mod tests;
