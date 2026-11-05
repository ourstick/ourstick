//! Signs that a folder may have changed: from the operating system through
//! `notify`, or on a timer when watching is not possible.

use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::event::{AccessKind, AccessMode};
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
pub struct Nudge(mpsc::UnboundedSender<()>);

impl Nudge {
    pub fn now(&self) {
        // Nobody to tell once watching has stopped.
        let _ = self.0.send(());
    }
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
    sender: mpsc::UnboundedSender<()>,
    receiver: mpsc::UnboundedReceiver<()>,
    watched: BTreeSet<PathBuf>,
    /// The same roots, for the system's callback.
    roots: std::sync::Arc<std::sync::Mutex<BTreeSet<PathBuf>>>,
}

/// Whether `path` is reached through a link below one of `roots`. A stick
/// never takes what is behind a link (the walk does not follow one), but
/// notify's recursive watch on Linux does: a link to a busy folder outside
/// the stick (Downloads, /tmp) would start a pass at every write there.
fn through_a_link(path: &Path, roots: &BTreeSet<PathBuf>) -> bool {
    let Some(root) = roots.iter().find(|root| path.starts_with(root)) else {
        return false;
    };
    path.ancestors()
        .skip(1)
        .take_while(|folder| *folder != root.as_path())
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
        let error = |reason: notify::Error| WatchError {
            path: root.display().to_string(),
            reason: reason.to_string(),
        };
        let (sender, receiver) = mpsc::unbounded_channel();
        let from_system = sender.clone();
        let roots =
            std::sync::Arc::new(std::sync::Mutex::new(BTreeSet::from([root.to_path_buf()])));
        let seen_roots = roots.clone();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let behind_links = event.as_ref().is_ok_and(|event| {
                let roots = seen_roots
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                !event.paths.is_empty()
                    && event.paths.iter().all(|path| through_a_link(path, &roots))
            });
            if is_change(&event) && !behind_links {
                // The receiver is gone only when watching has stopped.
                let _ = from_system.send(());
            }
        })
        .map_err(error)?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(error)?;
        Ok(Self {
            watcher,
            sender,
            receiver,
            watched: BTreeSet::from([root.to_path_buf()]),
            roots,
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

    fn watch_roots(&mut self, roots: &[PathBuf]) -> Vec<WatchError> {
        let wanted: BTreeSet<PathBuf> = roots.iter().cloned().collect();
        for gone in self
            .watched
            .difference(&wanted)
            .cloned()
            .collect::<Vec<_>>()
        {
            // A root that went is no longer watched by the system anyway.
            let _ = self.watcher.unwatch(&gone);
            self.watched.remove(&gone);
            self.roots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&gone);
        }
        let mut errors = Vec::new();
        for root in wanted
            .difference(&self.watched)
            .cloned()
            .collect::<Vec<_>>()
        {
            match self.watcher.watch(&root, RecursiveMode::Recursive) {
                Ok(()) => {
                    self.roots
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(root.clone());
                    self.watched.insert(root);
                }
                Err(reason) => errors.push(WatchError {
                    path: root.display().to_string(),
                    reason: reason.to_string(),
                }),
            }
        }
        errors
    }
}

/// A sign of change every `interval`, for when the folder cannot be watched.
pub struct PeriodicEvents {
    interval: tokio::time::Interval,
    sender: mpsc::UnboundedSender<()>,
    receiver: mpsc::UnboundedReceiver<()>,
}

impl PeriodicEvents {
    pub fn new(interval: Duration) -> Self {
        let start = tokio::time::Instant::now() + interval;
        let (sender, receiver) = mpsc::unbounded_channel();
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
mod tests {
    use super::*;

    #[test]
    fn reads_are_not_changes_but_writes_and_errors_are() {
        let access = |kind| Ok(Event::new(EventKind::Access(kind)));
        assert!(!is_change(&access(AccessKind::Open(AccessMode::Read))));
        assert!(!is_change(&access(AccessKind::Close(AccessMode::Read))));
        assert!(!is_change(&access(AccessKind::Read)));
        assert!(is_change(&access(AccessKind::Close(AccessMode::Write))));
        assert!(is_change(&Ok(Event::new(EventKind::Create(
            notify::event::CreateKind::File
        )))));
        assert!(is_change(&Ok(Event::new(EventKind::Remove(
            notify::event::RemoveKind::Any
        )))));
        assert!(is_change(&Err(notify::Error::generic("queue overflow"))));
    }

    #[cfg(unix)]
    #[test]
    fn a_path_reached_through_a_link_below_the_root_is_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("stick");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir(dir.path().join("busy")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("busy"), root.join("link")).unwrap();
        let roots = BTreeSet::from([root.clone()]);
        assert!(through_a_link(&root.join("link/log.txt"), &roots));
        assert!(through_a_link(&root.join("link/deep/x"), &roots));
        assert!(!through_a_link(&root.join("sub/a.txt"), &roots));
        // The link itself changing is the stick's own folder changing.
        assert!(!through_a_link(&root.join("link"), &roots));
        assert!(!through_a_link(&dir.path().join("elsewhere"), &roots));
    }

    #[tokio::test]
    async fn the_operating_system_reports_a_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut events = NotifyEvents::start(dir.path()).unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/new.txt"), b"x").unwrap();
        let seen = tokio::time::timeout(Duration::from_secs(5), events.changed()).await;
        assert_eq!(seen, Ok(Some(())));
    }

    #[tokio::test]
    async fn reading_a_file_is_not_reported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"x").unwrap();
        let mut events = NotifyEvents::start(dir.path()).unwrap();
        let _ = std::fs::read(dir.path().join("a.txt")).unwrap();
        let seen = tokio::time::timeout(Duration::from_millis(500), events.changed()).await;
        assert!(seen.is_err(), "a read was reported as a change");
    }

    #[tokio::test]
    async fn a_missing_folder_cannot_be_watched() {
        let dir = tempfile::tempdir().unwrap();
        let error = NotifyEvents::start(&dir.path().join("missing"))
            .err()
            .unwrap();
        assert!(error.to_string().contains("cannot watch"), "{error}");
    }

    #[tokio::test]
    async fn periodic_events_come_every_interval() {
        // Measured from before the interval starts: its ticks count from
        // there, and a busy machine may take a while to come back.
        let started = tokio::time::Instant::now();
        let mut events = PeriodicEvents::new(Duration::from_millis(100));
        events.changed().await;
        events.changed().await;
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(200), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(1_000), "{elapsed:?}");
    }
}
