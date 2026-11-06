//! Signs that a folder may have changed: from the operating system through
//! `notify`, or on a timer when watching is not possible.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::event::{AccessKind, AccessMode, ModifyKind};
use notify::{Event, EventKind};
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
    /// Awaited, never blocking: a share waiting for it can still stop.
    fn watch_roots(&mut self, roots: &[PathBuf]) -> impl Future<Output = Vec<WatchError>> + Send {
        let _ = roots;
        async { Vec::new() }
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
    sender: mpsc::Sender<()>,
    receiver: mpsc::Receiver<()>,
    /// The system's watch and what it watches, changed on a blocking
    /// thread: notify returns from watching a folder and all under it only
    /// once it has walked the whole tree (on Linux, a watch for each folder),
    /// which takes minutes for a big folder on a slow disk. A share waiting
    /// for that must still stop at once (Quit, Stop sending); the walk then
    /// ends after the folder it is in (`stopped`).
    watching: Arc<Mutex<Watching>>,
    /// Set when these events are dropped (the share stopped): the walk
    /// watches no further folder.
    stopped: Arc<AtomicBool>,
}

impl Drop for NotifyEvents {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
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
    /// watched again at the next look. So are the files sent alone whose
    /// folder went (a build that makes it again): its watch went with it.
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
                        .flat_map(|path| {
                            let folder_root =
                                roots.all.contains(path) && !roots.files.contains(path);
                            let files_in_it = roots
                                .files
                                .iter()
                                .filter(|file| file.parent() == Some(path.as_path()));
                            folder_root
                                .then_some(path)
                                .into_iter()
                                .chain(files_in_it)
                                .cloned()
                                .collect::<Vec<_>>()
                        })
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
        let stopped = Arc::new(AtomicBool::new(false));
        let mut watching = Watching {
            watcher,
            watched: BTreeMap::new(),
            targets: BTreeMap::new(),
            roots,
            stopped: stopped.clone(),
        };
        watching.add_root(root)?;
        Ok(Self {
            sender,
            receiver,
            watching: Arc::new(Mutex::new(watching)),
            stopped,
        })
    }

    /// A way to give this source a sign of change from outside.
    pub fn nudge(&self) -> Nudge {
        Nudge(self.sender.clone())
    }
}

impl FolderEvents for NotifyEvents {
    async fn changed(&mut self) -> Option<()> {
        self.receiver.recv().await
    }

    async fn watch_roots(&mut self, roots: &[PathBuf]) -> Vec<WatchError> {
        let watching = self.watching.clone();
        let wanted = roots.to_vec();
        match tokio::task::spawn_blocking(move || lock(&watching).watch_roots(&wanted)).await {
            Ok(unwatched) => unwatched,
            Err(failed) => roots
                .iter()
                .map(|root| WatchError {
                    path: root.display().to_string(),
                    reason: failed.to_string(),
                })
                .collect(),
        }
    }
}

/// `mutex`, also after a panic while it was held: what it guards stays
/// usable (sets of paths).
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

mod watching;
use watching::Watching;

#[cfg(test)]
mod tests;
