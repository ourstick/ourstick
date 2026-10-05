//! The owner's side of ADR 0017: for each version the owner publishes, the
//! files its own store serves; and, one at a time through the disks' turns
//! (ADR 0013), the files it cannot serve yet made ready (rule 7): their
//! plaintext hash and outboard, first those readers wait on, then the rest
//! in manifest order. It replaces the suitcases' keeper.

mod make;
mod version;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use self::make::{make, Made, ToMake};
use self::version::{load_version, Version};
use super::disk_turns::Disks;
use super::ShareError;
use crate::blocking::run_blocking;
use crate::cipher::{cipher_for, Cipher, StickKey};
use crate::layout::StickLayout;
use crate::manifest::hashing::outboards_dir;
use crate::stick_id::StickId;
use crate::store::Store;
use crate::transport::{BlobHash, Demand, Publisher, ServedFile};

/// How often the keeper looks whether the owner published another version
/// (a deletion, a rename or a linked folder brings no new blob to ask for).
const LOOK_EVERY: Duration = Duration::from_secs(2);
/// A file that could not be read is tried again after this, then after
/// twice as long each time, up to `LONGEST_RETRY`.
const FIRST_RETRY: Duration = Duration::from_secs(30);
const LONGEST_RETRY: Duration = Duration::from_secs(10 * 60);
/// A reader told "not ready" this recently is waiting: the file made for
/// it goes first on the disk (ADR 0013).
const READERS_WAIT_WINDOW: Duration = Duration::from_secs(60);

/// Asks the owner's watcher for a pass: a file changed since its version
/// and must be read again (ADR 0017, rule 4).
pub type PassWanted = Arc<dyn Fn() + Send + Sync>;

/// A [`PassWanted`] for a watcher started after the keeper: it nudges the
/// watcher once its [`Nudge`](crate::watcher::Nudge) is set in the cell
/// (before that, a change is seen by the watcher's own first look).
pub fn nudge_later() -> (PassWanted, Arc<std::sync::OnceLock<crate::watcher::Nudge>>) {
    let cell = Arc::new(std::sync::OnceLock::<crate::watcher::Nudge>::new());
    let nudging = cell.clone();
    let wanted: PassWanted = Arc::new(move || {
        if let Some(nudge) = nudging.get() {
            nudge.now();
        }
    });
    (wanted, cell)
}

/// Serves the files of stick `stick_id`, shared by `publisher` (a clone of
/// the one that shares it), from the owner's own store, and makes ready
/// those it cannot serve yet, until readers can no longer ask (the
/// publisher shut down). Only one per publisher; spawn it.
pub async fn keep_files<P: Publisher + 'static>(
    publisher: P,
    store: Arc<dyn Store>,
    stick_id: StickId,
    stick_key: StickKey,
    disks: Arc<Disks>,
    pass_wanted: PassWanted,
) -> Result<(), ShareError> {
    let mut demand = publisher.take_demand().ok_or(ShareError::KeeperRunning)?;
    let owned = {
        let store = store.clone();
        run_blocking(move || store.owned_stick(&stick_id)).await?
    }
    .ok_or(ShareError::NotOwned)?;
    let context = Context {
        store,
        stick_id,
        cipher: cipher_for(owned.cipher, &stick_key),
        cache_dir: publisher.cache_dir().to_path_buf(),
        outboards: outboards_dir(publisher.cache_dir()),
        disks,
        pass_wanted,
        stop: Arc::new(AtomicBool::new(false)),
    };
    let mut keeper = Keeper::default();
    keeper.reload(&publisher, &context).await;
    let mut job: Option<tokio::task::JoinHandle<(ToMake, Made)>> = None;
    let mut look = tokio::time::interval(LOOK_EVERY);
    let ended = loop {
        if job.is_none() {
            if let (Some(blob), Some(layout)) = (keeper.next(), keeper.layout.clone()) {
                let urgent = publisher.readers_waiting(READERS_WAIT_WINDOW);
                job = Some(start_making(&context, blob, layout, urgent));
            }
        }
        tokio::select! {
            event = demand.recv() => match event {
                Some(event) => keeper.on_demand(event, &publisher, &context).await,
                None => break Ok(()),
            },
            done = async { job.as_mut().expect("guarded").await }, if job.is_some() => {
                job = None;
                if let Ok((blob, made)) = done {
                    keeper.done(blob, made, &publisher, &context).await;
                }
            },
            _ = look.tick() => {
                keeper.retry_due();
                if keeper.version_changed(&context).await {
                    keeper.reload(&publisher, &context).await;
                }
            }
        }
    };
    // A file being made stops at its next disk window.
    context.stop.store(true, Ordering::Relaxed);
    ended
}

/// What the keeper works with.
struct Context {
    store: Arc<dyn Store>,
    stick_id: StickId,
    cipher: Arc<dyn Cipher>,
    cache_dir: PathBuf,
    outboards: PathBuf,
    disks: Arc<Disks>,
    pass_wanted: PassWanted,
    stop: Arc<AtomicBool>,
}

/// A blob of the version: its place (its first file's, in manifest order)
/// and every path it is at.
struct Known {
    place: usize,
    blob: ToMake,
}

#[derive(Default)]
struct Keeper {
    manifest_hash: Option<BlobHash>,
    layout: Option<StickLayout>,
    /// Every blob of the version.
    known: HashMap<BlobHash, Known>,
    /// Blobs to make, by place.
    pending: BTreeMap<usize, BlobHash>,
    /// Asked for while not ready, first first.
    wanted: VecDeque<usize>,
    /// Could not be read: when to try again, and the wait after that.
    failed: HashMap<BlobHash, (Instant, Duration)>,
}

impl Keeper {
    /// Whether the owner published another version since the last read.
    async fn version_changed(&self, context: &Context) -> bool {
        let store = context.store.clone();
        let stick_id = context.stick_id;
        let latest = run_blocking(move || store.owned_stick(&stick_id)).await;
        let latest = latest.ok().flatten().and_then(|owned| {
            owned
                .latest_record
                .map(|record| record.unverified_record().manifest_hash)
        });
        latest.is_some() && latest != self.manifest_hash
    }

    /// Reads the latest version and serves what it can of it.
    async fn reload(&mut self, publisher: &impl Publisher, context: &Context) {
        let loaded = {
            let store = context.store.clone();
            let stick_id = context.stick_id;
            let cache_dir = context.cache_dir.clone();
            let outboards = context.outboards.clone();
            let cipher = context.cipher.clone();
            run_blocking(move || {
                load_version(
                    store.as_ref(),
                    &stick_id,
                    &cache_dir,
                    &outboards,
                    cipher.as_ref(),
                )
            })
            .await
        };
        match loaded {
            Ok(Some(version)) if self.manifest_hash != Some(version.manifest_hash) => {
                self.take(version, publisher, context);
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "the current version could not be read"),
        }
    }

    fn take(&mut self, version: Version, publisher: &impl Publisher, context: &Context) {
        let Version {
            manifest_hash,
            served,
            entries,
        } = version;
        self.manifest_hash = Some(manifest_hash);
        self.layout = Some(served.layout.clone());
        self.known = HashMap::with_capacity(entries.len());
        self.pending = BTreeMap::new();
        for (place, (entry, is_served)) in entries.into_iter().enumerate() {
            if let Some(known) = self.known.get_mut(&entry.hash) {
                known.blob.paths.push(entry.path);
                continue;
            }
            if !is_served {
                self.pending.insert(place, entry.hash);
            }
            let blob = ToMake {
                hash: entry.hash,
                size: entry.size,
                paths: vec![entry.path],
            };
            self.known.insert(entry.hash, Known { place, blob });
        }
        self.wanted.clear();
        // A new version may have them at other paths: tried afresh.
        self.failed.clear();
        if !self.pending.is_empty() {
            tracing::debug!(files = self.pending.len(), "files to make ready");
        }
        publisher.serve_files(context.cipher.clone(), served);
    }

    /// The next blob to make: one a reader waits on, else the first.
    fn next(&mut self) -> Option<ToMake> {
        let hash = loop {
            match self.wanted.pop_front() {
                Some(place) => {
                    if let Some(hash) = self.pending.remove(&place) {
                        break hash;
                    }
                }
                None => break self.pending.pop_first()?.1,
            }
        };
        self.known.get(&hash).map(|known| known.blob.clone())
    }

    async fn on_demand(&mut self, event: Demand, publisher: &impl Publisher, context: &Context) {
        match event {
            Demand::Asked {
                hash,
                served: false,
                ..
            } => {
                let Some(place) = self.known.get(&hash).map(|known| known.place) else {
                    // A blob of a newer version.
                    if self.version_changed(context).await {
                        self.reload(publisher, context).await;
                    }
                    return;
                };
                // Waited on: made first, also if it failed lately.
                if self.failed.remove(&hash).is_some() {
                    self.pending.insert(place, hash);
                }
                if self.pending.contains_key(&place) {
                    self.wanted.retain(|wanted| *wanted != place);
                    self.wanted.push_front(place);
                }
            }
            // The store could not serve it (rule 4).
            Demand::NotServed { hash, changed } => {
                if changed {
                    self.changed(hash, context).await;
                } else {
                    self.failed_now(hash);
                }
            }
            Demand::Asked { .. } | Demand::Closed { .. } => {}
        }
    }

    /// Its file changed since the version: its records are forgotten and a
    /// pass is asked for, whose version lists it anew.
    async fn changed(&mut self, hash: BlobHash, context: &Context) {
        let Some(paths) = self.known.get(&hash).map(|known| known.blob.paths.clone()) else {
            return;
        };
        let store = context.store.clone();
        let stick_id = context.stick_id;
        let forgotten = run_blocking(move || store.forget_owned_files(&stick_id, &paths)).await;
        if let Err(error) = forgotten {
            tracing::warn!(%error, "a changed file's record could not be forgotten");
        }
        (context.pass_wanted)();
    }

    /// Could not be read: tried again later, waiting longer each time.
    fn failed_now(&mut self, hash: BlobHash) {
        if !self.known.contains_key(&hash) {
            return;
        }
        let wait = self
            .failed
            .get(&hash)
            .map_or(FIRST_RETRY, |(_, wait)| (*wait * 2).min(LONGEST_RETRY));
        self.failed.insert(hash, (Instant::now() + wait, wait));
    }

    /// The files whose wait is over, made again.
    fn retry_due(&mut self) {
        let now = Instant::now();
        for (hash, (at, _)) in &self.failed {
            if *at <= now {
                if let Some(known) = self.known.get(hash) {
                    self.pending.insert(known.place, *hash);
                }
            }
        }
    }

    /// A blob made, or not: served if it is the version's; its records
    /// forgotten if its file changed; tried again later if it could not be
    /// read.
    async fn done(
        &mut self,
        blob: ToMake,
        made: Made,
        publisher: &impl Publisher,
        context: &Context,
    ) {
        if !self.known.contains_key(&blob.hash) {
            // Of a version since replaced.
            return;
        }
        match made {
            Made::Ready(records) => {
                self.failed.remove(&blob.hash);
                let served = ServedFile {
                    path: blob.paths[0].as_str().into(),
                    plaintext_len: blob.size,
                    plaintext_hash: records[0].plaintext_hash.expect("made with one"),
                };
                let store = context.store.clone();
                let stick_id = context.stick_id;
                let recorded =
                    run_blocking(move || store.record_owned_files(&stick_id, &records)).await;
                if let Err(error) = recorded {
                    tracing::warn!(%error, "a file made ready could not be recorded");
                }
                publisher.serve_more(vec![(blob.hash, served)]);
            }
            Made::Changed => self.changed(blob.hash, context).await,
            Made::Unreadable(error) => {
                tracing::debug!(%error, "a file could not be made ready; tried again later");
                self.failed_now(blob.hash);
            }
        }
    }
}

fn start_making(
    context: &Context,
    blob: ToMake,
    layout: StickLayout,
    urgent: bool,
) -> tokio::task::JoinHandle<(ToMake, Made)> {
    let cipher = context.cipher.clone();
    let outboards = context.outboards.clone();
    let disks = context.disks.clone();
    let stop = context.stop.clone();
    tokio::task::spawn_blocking(move || {
        let made = make(
            &blob,
            &layout,
            cipher.as_ref(),
            &outboards,
            disks,
            urgent,
            stop,
        );
        (blob, made)
    })
}
