//! The owner's side of ADR 0017: for each version the owner publishes, the
//! files its own store serves; and, one at a time through the disks' turns
//! (ADR 0013), the files it cannot serve yet made ready (rule 7): their
//! plaintext hash and outboard, first those readers wait on, then the rest
//! in manifest order. It replaces the suitcases' keeper.

mod version;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use self::version::{load_version, Version};
use super::disk_turns::{DiskId, Disks, Readers};
use super::ShareError;
use crate::blocking::run_blocking;
use crate::cipher::{cipher_for, Cipher, StickKey};
use crate::layout::StickLayout;
use crate::manifest::hashing::{hash_file_telling_plaintext, outboards_dir};
use crate::manifest::{Entry, FileStamp};
use crate::stick_id::StickId;
use crate::store::{OwnedFile, Store};
use crate::transport::{BlobHash, Demand, Publisher, ServedFile};

/// The version is read again at most this often, for a blob it does not
/// know (a reader of a newer version).
const RELOAD_EVERY: Duration = Duration::from_secs(1);
/// A file that could not be read is tried again after this.
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// A reader told "not ready" this recently is waiting: the file made for
/// it goes first on the disk (ADR 0013).
const READERS_WAIT_WINDOW: Duration = Duration::from_secs(60);

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
    };
    let mut keeper = Keeper::default();
    keeper.reload(&publisher, &context).await;
    let mut job: Option<tokio::task::JoinHandle<(Entry, Made)>> = None;
    let mut retry = tokio::time::interval(RETRY_AFTER);
    loop {
        if job.is_none() {
            if let Some(entry) = keeper.next() {
                let urgent = publisher.readers_waiting(READERS_WAIT_WINDOW);
                job = Some(start_making(&context, entry, keeper.layout.clone(), urgent));
            }
        }
        tokio::select! {
            event = demand.recv() => match event {
                Some(event) => keeper.on_demand(event, &publisher, &context).await,
                None => {
                    if let Some(job) = job {
                        job.abort();
                    }
                    return Ok(());
                }
            },
            done = async { job.as_mut().expect("guarded").await }, if job.is_some() => {
                job = None;
                if let Ok((entry, made)) = done {
                    keeper.done(entry, made, &publisher, &context).await;
                }
            },
            _ = retry.tick() => keeper.retry(),
        }
    }
}

/// What the keeper works with.
struct Context {
    store: Arc<dyn Store>,
    stick_id: StickId,
    cipher: Arc<dyn Cipher>,
    cache_dir: PathBuf,
    outboards: PathBuf,
    disks: Arc<Disks>,
}

#[derive(Default)]
struct Keeper {
    manifest_hash: Option<BlobHash>,
    layout: Option<StickLayout>,
    /// Every file of the version, by blob hash, with its place in it.
    known: HashMap<BlobHash, (usize, Entry)>,
    /// Files to make, by place in the version.
    pending: BTreeMap<usize, BlobHash>,
    /// Asked for while not ready, first first.
    wanted: VecDeque<usize>,
    /// Could not be read; tried again at the next retry.
    failed: Vec<BlobHash>,
    last_reload: Option<Instant>,
}

impl Keeper {
    /// Reads the latest version and serves what it can of it.
    async fn reload(&mut self, publisher: &impl Publisher, context: &Context) {
        self.last_reload = Some(Instant::now());
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
            if !is_served {
                self.pending.insert(place, entry.hash);
            }
            self.known.insert(entry.hash, (place, entry));
        }
        self.wanted.clear();
        self.failed.clear();
        if !self.pending.is_empty() {
            tracing::debug!(files = self.pending.len(), "files to make ready");
        }
        publisher.serve_files(context.cipher.clone(), served);
    }

    /// The next file to make: one a reader waits on, else the first.
    fn next(&mut self) -> Option<Entry> {
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
        self.known.get(&hash).map(|(_, entry)| entry.clone())
    }

    async fn on_demand(&mut self, event: Demand, publisher: &impl Publisher, context: &Context) {
        match event {
            Demand::Asked {
                hash,
                served: false,
                ..
            } => match self.known.get(&hash) {
                Some(&(place, _)) => {
                    if self.pending.contains_key(&place) {
                        self.wanted.retain(|wanted| *wanted != place);
                        self.wanted.push_front(place);
                    }
                }
                // A blob of a newer version (or its manifest).
                None => {
                    if self
                        .last_reload
                        .is_none_or(|at| at.elapsed() >= RELOAD_EVERY)
                    {
                        self.reload(publisher, context).await;
                    }
                }
            },
            // The store could not serve it (rule 4): a changed file's record
            // is forgotten (the next pass reads it again); one that could
            // not be read is made again later.
            Demand::NotServed { hash, changed } => {
                let Some((_, entry)) = self.known.get(&hash).cloned() else {
                    return;
                };
                if changed {
                    forget(context, entry.path).await;
                } else {
                    self.failed.push(hash);
                }
            }
            Demand::Asked { .. } | Demand::Closed { .. } => {}
        }
    }

    /// The files that could not be read, tried again.
    fn retry(&mut self) {
        for hash in self.failed.drain(..) {
            if let Some(&(place, _)) = self.known.get(&hash) {
                self.pending.insert(place, hash);
            }
        }
    }

    /// A file made, or not: served if its blob is the version's; its record
    /// forgotten if the file changed; tried again later if it could not be
    /// read.
    async fn done(
        &mut self,
        entry: Entry,
        made: Made,
        publisher: &impl Publisher,
        context: &Context,
    ) {
        if !self.known.contains_key(&entry.hash) {
            // Of a version since replaced.
            return;
        }
        match made {
            Made::Ready(file) => {
                let served = ServedFile {
                    path: file.path.as_str().into(),
                    plaintext_len: entry.size,
                    plaintext_hash: file.plaintext_hash.expect("made with one"),
                };
                let store = context.store.clone();
                let stick_id = context.stick_id;
                let recorded =
                    run_blocking(move || store.record_owned_files(&stick_id, &[file])).await;
                if let Err(error) = recorded {
                    tracing::warn!(%error, "a file made ready could not be recorded");
                }
                publisher.serve_more(vec![(entry.hash, served)]);
            }
            Made::Changed => forget(context, entry.path).await,
            Made::Unreadable(error) => {
                tracing::debug!(%error, "a file could not be made ready; tried again later");
                self.failed.push(entry.hash);
            }
        }
    }
}

/// What making a file gave.
enum Made {
    /// Its record, with the plaintext hash, for the version's blob.
    Ready(OwnedFile),
    /// It is no longer what the version lists.
    Changed,
    Unreadable(std::io::Error),
}

/// Reads the file of `entry` through its disk's turn: its plaintext hash
/// and, if it has more than one group, its outboard into the cache. Kept
/// only if its blob is still the version's (rule 4).
fn start_making(
    context: &Context,
    entry: Entry,
    layout: Option<StickLayout>,
    urgent: bool,
) -> tokio::task::JoinHandle<(Entry, Made)> {
    let cipher = context.cipher.clone();
    let outboards = context.outboards.clone();
    let disks = context.disks.clone();
    tokio::task::spawn_blocking(move || {
        let made = make(
            &entry,
            layout.as_ref(),
            cipher.as_ref(),
            &outboards,
            disks,
            urgent,
        );
        (entry, made)
    })
}

fn make(
    entry: &Entry,
    layout: Option<&StickLayout>,
    cipher: &dyn Cipher,
    outboards: &std::path::Path,
    disks: Arc<Disks>,
    urgent: bool,
) -> Made {
    let Some(layout) = layout else {
        return Made::Unreadable(std::io::Error::other("no version"));
    };
    let path = layout.on_disk(&entry.path);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => return Made::Unreadable(error),
    };
    // Taken before the file is read: if it changes after, the next pass
    // reads it again.
    let stamp = FileStamp::of(&metadata);
    let mut readers = Readers::new(disks, urgent, || Ok(()));
    let reader = readers.on(DiskId::of_file(&path, &metadata));
    let hashed = hash_file_telling_plaintext(
        entry.path.clone(),
        &path,
        cipher,
        Some(reader),
        Some(outboards),
    );
    readers.end_windows();
    match hashed {
        Ok(hashed) if hashed.entry.hash == entry.hash => Made::Ready(OwnedFile {
            path: entry.path.clone(),
            stamp,
            blob_hash: entry.hash,
            plaintext_hash: Some(hashed.plaintext_hash),
        }),
        Ok(_) => Made::Changed,
        Err(crate::manifest::ManifestError::Blob {
            source: crate::cipher::CipherError::PlaintextChanged,
            ..
        }) => Made::Changed,
        Err(error) => Made::Unreadable(std::io::Error::other(error.to_string())),
    }
}

/// Forgets the record of the file at `path`: it changed, and the next pass
/// reads it again.
async fn forget(context: &Context, path: String) {
    let store = context.store.clone();
    let stick_id = context.stick_id;
    let forgotten = run_blocking(move || store.forget_owned_files(&stick_id, &[path])).await;
    if let Err(error) = forgotten {
        tracing::warn!(%error, "a changed file's record could not be forgotten");
    }
}
