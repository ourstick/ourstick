//! Signs that a folder may have changed: from the operating system through
//! `notify`, or on a timer when watching is not possible.

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use notify::event::{AccessKind, AccessMode};
use notify::{Event, EventKind, RecursiveMode, Watcher as _};
use tokio::sync::mpsc;

/// Tells when a folder may have changed. A sign may be spurious; the caller
/// checks what actually changed.
pub trait FolderEvents: Send {
    /// Waits for the next sign of change. `None` once there will be no more.
    fn changed(&mut self) -> impl Future<Output = Option<()>> + Send;
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
    _watcher: notify::RecommendedWatcher,
    receiver: mpsc::UnboundedReceiver<()>,
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
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            if is_change(&event) {
                // The receiver is gone only when watching has stopped.
                let _ = sender.send(());
            }
        })
        .map_err(error)?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(error)?;
        Ok(Self {
            _watcher: watcher,
            receiver,
        })
    }
}

impl FolderEvents for NotifyEvents {
    async fn changed(&mut self) -> Option<()> {
        self.receiver.recv().await
    }
}

/// A sign of change every `interval`, for when the folder cannot be watched.
pub struct PeriodicEvents {
    interval: tokio::time::Interval,
}

impl PeriodicEvents {
    pub fn new(interval: Duration) -> Self {
        let start = tokio::time::Instant::now() + interval;
        Self {
            interval: tokio::time::interval_at(start, interval),
        }
    }
}

impl FolderEvents for PeriodicEvents {
    async fn changed(&mut self) -> Option<()> {
        self.interval.tick().await;
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
        let mut events = PeriodicEvents::new(Duration::from_millis(100));
        let started = tokio::time::Instant::now();
        events.changed().await;
        events.changed().await;
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(200), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(1_000), "{elapsed:?}");
    }
}
