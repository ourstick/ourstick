//! Keeping a stick in step with its folder: wait for a sign of change, let
//! the folder go quiet, make sure no file is still being written, then run a
//! pass (which reads only what changed and signs a new version only if the
//! manifest did change).

mod events;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use events::{FolderEvents, NotifyEvents, PeriodicEvents, WatchError};

pub use crate::share::Owner;

use crate::blocking::run_blocking;
use crate::manifest::build::regular_files;
use crate::manifest::{FileStamp, Manifest};
use crate::share::{run_pass, PassOptions, Publication, ShareError};
use crate::transport::Publisher;

/// How long to wait at each stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchTiming {
    /// No sign of change for this long before publishing.
    pub quiet: Duration,
    /// Every file's size and times must stay the same over this long.
    pub settle: Duration,
    /// After a failed publish or a folder still being written, try again
    /// after this long even without a new sign of change.
    pub retry: Duration,
}

impl Default for WatchTiming {
    fn default() -> Self {
        Self {
            quiet: Duration::from_secs(2),
            settle: Duration::from_secs(1),
            retry: Duration::from_secs(30),
        }
    }
}

/// Which files differ between two manifests, by path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSummary {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
}

impl ChangeSummary {
    pub fn between(old: &Manifest, new: &Manifest) -> Self {
        let old: BTreeMap<&str, [u8; 32]> = old
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.hash))
            .collect();
        let new: BTreeMap<&str, [u8; 32]> = new
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.hash))
            .collect();
        let mut summary = Self::default();
        for (path, hash) in &new {
            match old.get(path) {
                None => summary.added.push((*path).to_owned()),
                Some(before) if before != hash => summary.changed.push((*path).to_owned()),
                Some(_) => {}
            }
        }
        let new_paths: BTreeSet<&&str> = new.keys().collect();
        summary.removed = old
            .keys()
            .filter(|path| !new_paths.contains(path))
            .map(|path| (*path).to_owned())
            .collect();
        summary
    }
}

/// What the watcher did, for display.
#[derive(Debug)]
pub enum WatchEvent<'a> {
    /// A file is still being written; publishing waits.
    StillChanging,
    /// A new version was published.
    Published {
        changes: &'a ChangeSummary,
        publication: &'a Publication,
    },
    /// Signs of change, but the stick is as last published.
    Unchanged,
    /// Publishing failed; tried again at the next change, or after `retry`.
    Failed { error: &'a ShareError },
    /// The stick's drive is not there (ejected): the last version is still
    /// served, and publishing resumes when it is back. Reported once.
    Ejected,
}

/// Every file's stamp under `root`, or `None` if the folder cannot be read
/// right now (e.g. a file vanished during the walk).
fn stamps(root: &Path) -> Option<BTreeMap<PathBuf, FileStamp>> {
    let files = regular_files(root).ok()?;
    files
        .into_iter()
        .map(|path| {
            let stamp = FileStamp::of(&std::fs::metadata(&path).ok()?);
            Some((path, stamp))
        })
        .collect()
}

/// True if no file under `root` changes size or times over `settle`.
pub async fn is_settled(root: &Path, settle: Duration) -> bool {
    let read = |root: PathBuf| run_blocking(move || Ok::<_, std::io::Error>(stamps(&root)));
    let Ok(Some(before)) = read(root.to_path_buf()).await else {
        return false;
    };
    tokio::time::sleep(settle).await;
    matches!(read(root.to_path_buf()).await, Ok(Some(after)) if after == before)
}

/// Waits for a sign of change, or with `retry_after`, for that long at most.
async fn next_change(events: &mut impl FolderEvents, retry_after: Option<Duration>) -> bool {
    match retry_after {
        None => events.changed().await.is_some(),
        Some(retry) => match tokio::time::timeout(retry, events.changed()).await {
            Ok(change) => change.is_some(),
            Err(_) => true,
        },
    }
}

/// Publishes `root` again whenever it changes, until `events` ends. `current`
/// is the manifest being served now, to report what changed.
pub async fn follow_folder(
    owner: &Owner<'_, impl Publisher>,
    root: &Path,
    events: &mut impl FolderEvents,
    timing: WatchTiming,
    mut current: Manifest,
    on_event: &mut (dyn FnMut(WatchEvent<'_>) + Send),
) {
    let mut retry = false;
    let mut ejected = false;
    loop {
        if !next_change(events, retry.then_some(timing.retry)).await {
            return;
        }
        // Let a burst of changes (a copy, an unzip) finish first.
        while let Ok(Some(())) = tokio::time::timeout(timing.quiet, events.changed()).await {}
        if !is_settled(root, timing.settle).await {
            on_event(WatchEvent::StillChanging);
            retry = true;
            continue;
        }
        let passed = run_pass(owner, PassOptions::default(), &mut |_| {}).await;
        retry = false;
        let published = match passed {
            Ok(outcome) => {
                ejected = false;
                // A file changed or went while it was read: another pass soon.
                retry = outcome.skipped > 0;
                Ok(outcome.publication)
            }
            Err(error) => Err(error),
        };
        match published {
            Err(ShareError::DriveMissing { .. }) => {
                if !ejected {
                    ejected = true;
                    on_event(WatchEvent::Ejected);
                }
                retry = true;
            }
            Ok(publication) if publication.new_version => {
                let changes = ChangeSummary::between(&current, &publication.manifest);
                on_event(WatchEvent::Published {
                    changes: &changes,
                    publication: &publication,
                });
                current = publication.manifest;
            }
            Ok(_) => on_event(WatchEvent::Unchanged),
            Err(error) => {
                tracing::warn!(%error, "publishing a change failed");
                on_event(WatchEvent::Failed { error: &error });
                retry = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Entry;

    fn manifest(entries: &[(&str, u8)]) -> Manifest {
        Manifest {
            entries: entries
                .iter()
                .map(|(path, hash)| Entry {
                    path: (*path).to_owned(),
                    size: 1,
                    mtime: 0,
                    hash: [*hash; 32],
                })
                .collect(),
            ..Manifest::default()
        }
    }

    #[test]
    fn summary_lists_added_changed_and_removed_paths() {
        let old = manifest(&[("a", 1), ("b", 2), ("c", 3)]);
        let new = manifest(&[("a", 1), ("b", 9), ("d", 4)]);
        assert_eq!(
            ChangeSummary::between(&old, &new),
            ChangeSummary {
                added: vec!["d".to_owned()],
                changed: vec!["b".to_owned()],
                removed: vec!["c".to_owned()],
            }
        );
        assert_eq!(ChangeSummary::between(&new, &new), ChangeSummary::default());
    }

    #[tokio::test]
    async fn a_file_being_written_is_not_settled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("growing.bin");
        std::fs::write(&path, b"").unwrap();
        assert!(is_settled(dir.path(), Duration::from_millis(50)).await);

        let writer = std::thread::spawn(move || {
            use std::io::Write;
            let mut file = std::fs::File::options().append(true).open(path).unwrap();
            for _ in 0..20 {
                file.write_all(&[0; 1024]).unwrap();
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        assert!(!is_settled(dir.path(), Duration::from_millis(100)).await);
        writer.join().unwrap();
        assert!(is_settled(dir.path(), Duration::from_millis(50)).await);
    }
}
