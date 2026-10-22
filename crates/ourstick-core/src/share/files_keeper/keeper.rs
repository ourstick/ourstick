//! The keeper's state: the version's blobs, those to make (wanted ones
//! first), and those that could not be read, tried again later.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::make::{Made, ToMake};
use super::version::{load_version, Listed, Version};
use super::{Context, FIRST_RETRY, LONGEST_RETRY};
use crate::blocking::run_blocking;
use crate::layout::StickLayout;
use crate::manifest::{FileStamp, Manifest};
use crate::transport::{BlobHash, Demand, PublishedVersion, Publisher, ServedFile};

/// The keeper's state, by entry index into the version's manifest (ADR
/// 0019): a blob's place is its first entry's index.
#[derive(Default)]
pub(super) struct Keeper {
    pub(super) manifest_hash: Option<BlobHash>,
    pub(super) layout: Option<StickLayout>,
    /// The version's manifest, shared with the owner's store.
    manifest: Option<Arc<Manifest>>,
    /// The stamp the catalogue recorded with each entry's blob, by index.
    stamps: Vec<Option<FileStamp>>,
    /// Every blob of the version: its first entry's index.
    known: HashMap<BlobHash, u32>,
    /// The other entries of blobs at more than one path (identical files).
    copies: HashMap<BlobHash, Vec<u32>>,
    /// Blobs to make, by place.
    pending: BTreeMap<usize, BlobHash>,
    /// Asked for while not ready, first first.
    wanted: VecDeque<usize>,
    /// Could not be read: when it was last tried, and the wait after that.
    failed: HashMap<BlobHash, (Instant, Duration)>,
    /// Asked for before its version came (a reader quicker than the
    /// keeper): wanted once it comes. At most `EARLY_ASKS`.
    early: VecDeque<BlobHash>,
    /// The version whose records could not be read last, told once.
    unreadable: Option<BlobHash>,
}

/// How many blobs asked for before their version the keeper remembers.
const EARLY_ASKS: usize = 64;

impl Keeper {
    /// Serves what it can of `published`, unless it is the version it has.
    /// If its records cannot be read, it stays the old one until asked again.
    pub(super) async fn reload(
        &mut self,
        published: Option<PublishedVersion>,
        publisher: &impl Publisher,
        context: &Context,
    ) {
        let Some(published) = published else {
            return;
        };
        let hash = published.manifest_hash;
        if self.manifest_hash == Some(hash) {
            return;
        }
        let loaded = {
            let store = context.store.clone();
            let stick_id = context.stick_id;
            let outboards = context.outboards.clone();
            let cipher = context.cipher.clone();
            run_blocking(move || {
                load_version(
                    store.as_ref(),
                    &stick_id,
                    &outboards,
                    cipher.as_ref(),
                    published,
                )
            })
            .await
        };
        match loaded {
            Ok(version) => {
                self.unreadable = None;
                self.take(version, publisher, context);
            }
            // Tried again at the next look; told once.
            Err(error) if self.unreadable == Some(hash) => {
                tracing::debug!(%error, "the current version still could not be read");
            }
            Err(error) => {
                self.unreadable = Some(hash);
                tracing::warn!(%error, "the current version could not be read");
            }
        }
    }

    fn take(&mut self, version: Version, publisher: &impl Publisher, context: &Context) {
        let Version {
            manifest_hash,
            served,
            listed,
        } = version;
        self.learn(
            manifest_hash,
            served.layout.clone(),
            served.manifest.clone(),
            &listed,
        );
        publisher.serve_files(context.cipher.clone(), served);
    }

    /// The version `manifest_hash`, its `manifest` with its files at
    /// `layout`, and what was recorded of each entry.
    fn learn(
        &mut self,
        manifest_hash: BlobHash,
        layout: StickLayout,
        manifest: Arc<Manifest>,
        listed: &[Listed],
    ) {
        self.manifest_hash = Some(manifest_hash);
        self.layout = Some(layout);
        self.known = HashMap::with_capacity(manifest.entries.len());
        self.copies = HashMap::new();
        self.pending = BTreeMap::new();
        for (index, (entry, listed)) in manifest.entries.iter().zip(listed).enumerate() {
            let index = u32::try_from(index).expect("fewer than 2^32 entries");
            if let Some(first) = self.known.get(&entry.hash) {
                // Served from a copy: nothing to make.
                if listed.served {
                    self.pending.remove(&(*first as usize));
                }
                self.copies.entry(entry.hash).or_default().push(index);
                continue;
            }
            if !listed.served {
                self.pending.insert(index as usize, entry.hash);
            }
            self.known.insert(entry.hash, index);
        }
        self.stamps = listed.iter().map(|listed| listed.stamp).collect();
        self.manifest = Some(manifest);
        self.wanted.clear();
        // A new version may have them at other paths: tried afresh.
        self.failed.clear();
        for hash in std::mem::take(&mut self.early) {
            self.asked(hash);
        }
        if !self.pending.is_empty() {
            tracing::debug!(files = self.pending.len(), "files to make ready");
        }
    }

    /// The next blob to make: one a reader waits on, else the first.
    pub(super) fn next(&mut self) -> Option<ToMake> {
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
        self.to_make(&hash)
    }

    /// What making `hash` needs: its entries' paths and stamps, from the
    /// manifest, made now rather than kept for every blob.
    fn to_make(&self, hash: &BlobHash) -> Option<ToMake> {
        let manifest = self.manifest.as_ref()?;
        let first = *self.known.get(hash)?;
        let indexes =
            std::iter::once(first).chain(self.copies.get(hash).into_iter().flatten().copied());
        let (paths, stamps) = indexes
            .map(|index| {
                (
                    manifest.entries[index as usize].path.clone(),
                    self.stamps.get(index as usize).copied().flatten(),
                )
            })
            .unzip();
        Some(ToMake {
            hash: *hash,
            index: first,
            paths,
            stamps,
        })
    }

    pub(super) async fn on_demand(&mut self, event: Demand, context: &Context) {
        match event {
            Demand::Asked {
                hash,
                served: false,
                ..
            } => {
                self.asked_maybe_early(hash);
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

    /// A reader waits on `hash`; if it is not known, it may be of a version
    /// not come yet (it comes as it is published): wanted once it comes.
    fn asked_maybe_early(&mut self, hash: BlobHash) {
        if !self.asked(hash) && !self.early.contains(&hash) {
            if self.early.len() == EARLY_ASKS {
                self.early.pop_front();
            }
            self.early.push_back(hash);
        }
    }

    /// A reader waits on `hash`: made first; one that failed, once its first
    /// wait is over (a reader asks every few seconds). False if it is not
    /// of this version.
    fn asked(&mut self, hash: BlobHash) -> bool {
        let Some(place) = self.known.get(&hash).map(|index| *index as usize) else {
            return false;
        };
        if let Some((tried, _)) = self.failed.get_mut(&hash) {
            if tried.elapsed() >= FIRST_RETRY {
                *tried = Instant::now();
                self.pending.insert(place, hash);
            }
        }
        if self.pending.contains_key(&place) {
            self.wanted.retain(|wanted| *wanted != place);
            self.wanted.push_front(place);
        }
        true
    }

    /// Its file changed since the version: its records are forgotten and a
    /// pass is asked for, whose version lists it anew.
    async fn changed(&mut self, hash: BlobHash, context: &Context) {
        let Some(paths) = self.to_make(&hash).map(|blob| blob.paths) else {
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
        self.failed.insert(hash, (Instant::now(), wait));
    }

    /// The files whose wait is over, made again. Each counts as tried now,
    /// so a retry under way is not queued once more meanwhile; the wait
    /// itself grows only if it fails.
    /// Whether something waits to be tried again: a file that could not
    /// be read, or the version's records.
    pub(super) fn has_to_look(&self) -> bool {
        !self.failed.is_empty() || self.unreadable.is_some()
    }

    pub(super) fn retry_due(&mut self) {
        let now = Instant::now();
        for (hash, (tried, wait)) in &mut self.failed {
            if now.duration_since(*tried) >= *wait {
                *tried = now;
                if let Some(index) = self.known.get(hash) {
                    self.pending.insert(*index as usize, *hash);
                }
            }
        }
    }

    /// A blob made, or not: served if it is the version's; its records
    /// forgotten if its file changed; tried again later if it could not be
    /// read.
    pub(super) async fn done(
        &mut self,
        blob: ToMake,
        made: Made,
        publisher: &impl Publisher,
        context: &Context,
    ) {
        match made {
            Made::Ready(records) => {
                self.failed.remove(&blob.hash);
                let served = ServedFile {
                    index: blob.index,
                    plaintext_hash: records[0].plaintext_hash.expect("made with one"),
                };
                let store = context.store.clone();
                let stick_id = context.stick_id;
                let recorded =
                    run_blocking(move || store.record_owned_files(&stick_id, &records)).await;
                if let Err(error) = recorded {
                    tracing::warn!(%error, "a file made ready could not be recorded");
                }
                if let Some(manifest) = &self.manifest {
                    publisher.serve_more(manifest, vec![(blob.hash, served)]);
                }
            }
            Made::Changed => self.changed(blob.hash, context).await,
            Made::Unreadable(error) => {
                tracing::debug!(%error, "a file could not be made ready; tried again later");
                self.failed_now(blob.hash);
            }
        }
    }
}

#[cfg(test)]
mod tests;
