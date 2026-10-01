//! Keeping a stick in step with its folder: wait for a sign of change, let
//! the folder go quiet, make sure no file is still being written, then run a
//! pass (which reads only what changed and signs a new version only if the
//! manifest did change).

mod events;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use events::{FolderEvents, NotifyEvents, Nudge, PeriodicEvents, WatchError};

pub use crate::share::Owner;

use crate::blocking::run_blocking;
use crate::layout::StickLayout;
use crate::manifest::build::regular_files;
use crate::manifest::{FileStamp, Manifest};
use crate::share::{
    run_pass, PartialTiming, PassEvent, PassOptions, Publication, ShareError, PARTIAL_TIMING,
};
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
    /// When a long pass publishes partial versions (ADR 0007); `None`: only
    /// at its end.
    pub partial: Option<PartialTiming>,
}

impl Default for WatchTiming {
    fn default() -> Self {
        Self {
            quiet: Duration::from_secs(2),
            settle: Duration::from_secs(1),
            retry: Duration::from_secs(30),
            partial: Some(PARTIAL_TIMING),
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
    /// `done` of `total` files are read, during a pass.
    Reading { done: usize, total: usize },
    /// A new version was published (a partial one while a pass reads the
    /// folder, `publication.manifest.partial`).
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
    /// Some of the stick's folders cannot be watched (e.g. the system's
    /// limit on watches is reached): their changes are seen by looking
    /// again every `retry`. Reported when it starts.
    SeenSlowly,
    /// Linked entries that are gone, and own top-level names left out for
    /// a linked one's, as a pass found them ([`PassEvent::Roots`]).
    Roots {
        gone: &'a [String],
        left_out: &'a [String],
    },
}

/// Every file's stamp under each of `roots` (a root may be one file), or
/// `None` if one cannot be read right now (e.g. a file vanished during the
/// walk).
fn stamps(roots: &[PathBuf]) -> Option<BTreeMap<PathBuf, FileStamp>> {
    let mut all = BTreeMap::new();
    for root in roots {
        let metadata = std::fs::metadata(root).ok()?;
        let files = if metadata.is_file() {
            vec![root.clone()]
        } else {
            regular_files(root).ok()?
        };
        for path in files {
            let stamp = FileStamp::of(&std::fs::metadata(&path).ok()?);
            all.insert(path, stamp);
        }
    }
    Some(all)
}

/// True if no file under `root` changes size or times over `settle`.
pub async fn is_settled(root: &Path, settle: Duration) -> bool {
    are_settled(vec![root.to_path_buf()], settle).await
}

/// [`is_settled`] for several roots at once.
pub async fn are_settled(roots: Vec<PathBuf>, settle: Duration) -> bool {
    let read = |roots: Vec<PathBuf>| run_blocking(move || Ok::<_, std::io::Error>(stamps(&roots)));
    let Ok(Some(before)) = read(roots.clone()).await else {
        return false;
    };
    tokio::time::sleep(settle).await;
    matches!(read(roots).await, Ok(Some(after)) if after == before)
}

/// The stick's roots that are there now, and whether any linked one is
/// gone. Its own folder always counts (an ejected drive is the pass's to
/// tell).
async fn present_roots(owner: &Owner<'_, impl Publisher>) -> (Vec<PathBuf>, bool) {
    let store = owner.store.clone();
    let stick_id = owner.signing_key.stick_id();
    let found = run_blocking(move || {
        let layout = StickLayout::load(store.as_ref(), &stick_id)?;
        let mut present = vec![layout.own().path.clone()];
        let mut any_gone = false;
        for root in layout.linked() {
            if root.is_gone() {
                any_gone = true;
            } else {
                present.push(root.path.clone());
            }
        }
        Ok::<_, crate::store::StoreError>((present, any_gone))
    })
    .await;
    match found {
        Ok(found) => found,
        Err(error) => {
            tracing::warn!(%error, "the stick's folders could not be read from the database");
            (Vec::new(), true)
        }
    }
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
    events: &mut impl FolderEvents,
    timing: WatchTiming,
    current: Manifest,
    on_event: &mut (dyn FnMut(WatchEvent<'_>) + Send),
) {
    follow(owner, events, timing, current, false, on_event).await;
}

/// Like [`follow_folder`], with a pass at once: after a start that served
/// the last version without reading the folder
/// ([`crate::share::serve_latest`]).
pub async fn follow_folder_reading_first(
    owner: &Owner<'_, impl Publisher>,
    events: &mut impl FolderEvents,
    timing: WatchTiming,
    current: Manifest,
    on_event: &mut (dyn FnMut(WatchEvent<'_>) + Send),
) {
    follow(owner, events, timing, current, true, on_event).await;
}

async fn follow(
    owner: &Owner<'_, impl Publisher>,
    events: &mut impl FolderEvents,
    timing: WatchTiming,
    mut current: Manifest,
    pass_first: bool,
    on_event: &mut (dyn FnMut(WatchEvent<'_>) + Send),
) {
    let mut retry = false;
    let mut ejected = false;
    let mut slow = false;
    let mut pass_now = pass_first;
    loop {
        // Watch what is there now: linked entries come and go (ADR 0008).
        // One gone, or one that cannot be watched, is looked at again every
        // `retry`.
        let (roots, any_gone) = present_roots(owner).await;
        let unwatched = events.watch_roots(&roots);
        for error in &unwatched {
            tracing::warn!(%error, "a folder of the stick cannot be watched");
        }
        if !unwatched.is_empty() && !slow {
            on_event(WatchEvent::SeenSlowly);
        }
        slow = !unwatched.is_empty();
        let look_again = retry || any_gone || slow;
        if !std::mem::take(&mut pass_now)
            && !next_change(events, look_again.then_some(timing.retry)).await
        {
            return;
        }
        // Let a burst of changes (a copy, an unzip) finish first.
        while let Ok(Some(())) = tokio::time::timeout(timing.quiet, events.changed()).await {}
        let (roots, _) = present_roots(owner).await;
        if !are_settled(roots, timing.settle).await {
            on_event(WatchEvent::StillChanging);
            retry = true;
            continue;
        }
        let options = PassOptions {
            rescan: false,
            partial: timing.partial,
        };
        let passed = run_pass(owner, options, &mut |event| match event {
            PassEvent::Progress { done, total } => on_event(WatchEvent::Reading { done, total }),
            PassEvent::Roots { gone, left_out } => on_event(WatchEvent::Roots { gone, left_out }),
            PassEvent::Published(publication) => {
                let changes = ChangeSummary::between(&current, &publication.manifest);
                on_event(WatchEvent::Published {
                    changes: &changes,
                    publication,
                });
                current = publication.manifest.clone();
            }
        })
        .await;
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
mod tests;
