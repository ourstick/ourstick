//! The suitcases' keeper (ADR 0007): the owner's policy for what is served
//! from where. Readers ask for blobs; a blob no suitcase holds is "not
//! ready", and its suitcase is prepared, first come first served. While a
//! reader is on one suitcase the next one is prepared (read-ahead). The
//! suitcases take at most the budget: to make room, the one used longest ago
//! leaves, but never the one a reader is on nor its next one; a reader that
//! asked nothing for [`IN_USE`] holds none. With no room at all, readers
//! wait ("not ready") until some is free.

mod budget;
mod plan;
mod prepare;

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use budget::{Budget, MAX_BUDGET, MIN_BUDGET};

use self::plan::{plan, Plan, SUITCASE_BYTES};
use self::prepare::{prepare, FileJob, Prepared};
use super::latest::load_latest_manifest;
use super::ShareError;
use crate::blob::decrypt_blob;
use crate::blocking::run_blocking;
use crate::cipher::{cipher_for, Cipher, StickKey};
use crate::manifest::Manifest;
use crate::stick_id::StickId;
use crate::store::Store;
use crate::transport::{BlobHash, Demand, Publisher, SuitcaseId};

/// A reader holds the suitcase it is on, and the next one, while it asked
/// for something this recently.
pub const IN_USE: Duration = Duration::from_secs(30);
/// With no room, the keeper looks again this often.
const NO_ROOM_RETRY: Duration = Duration::from_millis(500);
/// The current version is looked up again at most this often, for a blob
/// the keeper does not know.
const RELOAD_EVERY: Duration = Duration::from_secs(1);

/// Keeps the suitcases of stick `stick_id`, served by `publisher` (a clone
/// of the one that shares it), within `budget`, until readers can no longer
/// ask (the publisher shut down). Only one keeper per publisher; spawn it.
pub async fn keep_suitcases<P: Publisher + 'static>(
    publisher: P,
    store: Arc<dyn Store>,
    stick_id: StickId,
    stick_key: StickKey,
    budget: Budget,
) -> Result<(), ShareError> {
    let mut demand = publisher.take_demand().ok_or(ShareError::KeeperRunning)?;
    let owned = {
        let store = store.clone();
        run_blocking(move || store.owned_stick(&stick_id)).await?
    }
    .ok_or(ShareError::NotOwned)?;
    let context = Context {
        cache_dir: publisher.cache_dir().to_path_buf(),
        store,
        stick_id,
        cipher: cipher_for(owned.cipher, &stick_key),
        root: owned.root_path,
        suitcase_bytes: SUITCASE_BYTES.min(budget.total() / 6).max(1),
    };
    let publisher = &publisher;
    let mut state = State::default();
    let mut job: Option<Pin<Box<dyn Future<Output = Done> + Send + '_>>> = None;
    let mut retry_at: Option<tokio::time::Instant> = None;
    loop {
        if job.is_none() && retry_at.is_none_or(|at| tokio::time::Instant::now() >= at) {
            retry_at = None;
            match state.next_job(&budget, Instant::now()) {
                Next::Job(next) => {
                    job = Some(Box::pin(run_job(
                        publisher,
                        context.cipher.clone(),
                        budget.clone(),
                        Forget {
                            store: context.store.clone(),
                            stick_id: context.stick_id,
                        },
                        next,
                    )))
                }
                Next::NoRoom => retry_at = Some(tokio::time::Instant::now() + NO_ROOM_RETRY),
                Next::Nothing => {}
            }
        }
        tokio::select! {
            event = demand.recv() => match event {
                Some(event) => state.on_demand(event, &context).await,
                None => return Ok(()),
            },
            done = async { job.as_mut().expect("guarded").await }, if job.is_some() => {
                job = None;
                if state.done(done) {
                    retry_at = Some(tokio::time::Instant::now() + NO_ROOM_RETRY);
                }
            },
            () = async { tokio::time::sleep_until(retry_at.expect("guarded")).await },
                if retry_at.is_some() && job.is_none() => {}
        }
    }
}

/// What the keeper works with.
struct Context {
    cache_dir: PathBuf,
    store: Arc<dyn Store>,
    stick_id: StickId,
    cipher: Arc<dyn Cipher>,
    root: PathBuf,
    suitcase_bytes: u64,
}

/// The version being served, cut into suitcases.
struct Current {
    manifest_hash: BlobHash,
    plan: Plan,
    files: Vec<FileJob>,
}

/// A suitcase that is open.
#[derive(Debug)]
struct Open {
    id: SuitcaseId,
    /// Its place in the version it was cut from: (generation, number).
    place: (u64, usize),
    bytes: u64,
    last_used: Instant,
    ready: HashSet<BlobHash>,
}

/// A suitcase to prepare, after the ones to evict.
struct Job {
    place: (u64, usize),
    files: Vec<FileJob>,
    bytes: u64,
    evict: Vec<(SuitcaseId, u64)>,
    /// Bigger than the whole budget: taken once nothing else is left.
    beyond: bool,
}

/// How a job ended.
struct Done {
    place: (u64, usize),
    bytes: u64,
    result: Result<Option<Prepared>, ShareError>,
}

enum Next {
    Job(Job),
    NoRoom,
    Nothing,
}

#[derive(Default)]
struct State {
    current: Option<Current>,
    /// Bumped with every new version; places of earlier ones are stale.
    generation: u64,
    open: Vec<Open>,
    preparing: Option<(u64, usize)>,
    /// Suitcases to prepare, in turn (numbers in the current version).
    queue: VecDeque<usize>,
    /// Where each reader is (by connection), and when it last asked.
    readers: HashMap<u64, (Place, Instant)>,
    last_reload: Option<Instant>,
}

/// A suitcase's place: (generation of the version, number in it).
type Place = (u64, usize);

impl State {
    fn is_underway(&self, number: usize) -> bool {
        let place = (self.generation, number);
        self.preparing == Some(place)
            || self.open.iter().any(|open| open.place == place)
            || self.queue.contains(&number)
    }

    async fn on_demand(&mut self, event: Demand, context: &Context) {
        let (connection, hash, served) = match event {
            Demand::Closed { connection } => {
                self.readers.remove(&connection);
                return;
            }
            Demand::Asked {
                connection,
                hash,
                served,
            } => (connection, hash, served),
        };
        let known = |state: &Self| {
            state
                .current
                .as_ref()
                .is_some_and(|current| current.plan.by_hash.contains_key(&hash))
        };
        if !known(self)
            && self
                .last_reload
                .is_none_or(|at| at.elapsed() >= RELOAD_EVERY)
        {
            self.last_reload = Some(Instant::now());
            match load_current(context).await {
                Ok(Some(current))
                    if self
                        .current
                        .as_ref()
                        .is_none_or(|old| old.manifest_hash != current.manifest_hash) =>
                {
                    // A new version: the queue and places refer to the old
                    // one; open suitcases keep serving what they hold.
                    self.generation += 1;
                    self.queue.clear();
                    self.current = Some(current);
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "the current version could not be read"),
            }
        }
        self.note(connection, hash, served, Instant::now());
    }

    /// Takes in that `connection` asked for `hash` at `now`, once the
    /// version is known.
    fn note(&mut self, connection: u64, hash: BlobHash, served: bool, now: Instant) {
        let Some(first) = self
            .current
            .as_ref()
            .and_then(|current| current.plan.by_hash.get(&hash))
            .and_then(|places| places.first().copied())
        else {
            return;
        };
        let generation = self.generation;
        let mut place = (generation, first);
        if served {
            let mut found = None;
            for open in self
                .open
                .iter_mut()
                .filter(|open| open.ready.contains(&hash))
            {
                open.last_used = now;
                if found.is_none() || open.place.0 == generation {
                    found = Some(open.place);
                }
            }
            place = found.unwrap_or(place);
        }
        self.readers.insert(connection, (place, now));
        if served {
            // Read-ahead: the next suitcase, one per reader.
            let next = place.1 + 1;
            let exists = self
                .current
                .as_ref()
                .is_some_and(|current| next < current.plan.suitcases.len());
            if place.0 == generation && exists && !self.is_underway(next) {
                self.queue.push_back(next);
            }
        } else if !self.is_underway(first) {
            self.queue.push_front(first);
        }
    }

    /// Whether a reader holds the suitcase at `place`: it is on it, or on
    /// the one before, and asked for something within [`IN_USE`].
    fn is_held(&self, place: Place, now: Instant) -> bool {
        self.readers.values().any(|(at, asked)| {
            now.duration_since(*asked) < IN_USE
                && at.0 == place.0
                && (at.1 == place.1 || at.1 + 1 == place.1)
        })
    }

    /// The next suitcase to prepare, and what must leave for it.
    fn next_job(&mut self, budget: &Budget, now: Instant) -> Next {
        let Some(current) = &self.current else {
            return Next::Nothing;
        };
        let generation = self.generation;
        let number = loop {
            let Some(number) = self.queue.pop_front() else {
                return Next::Nothing;
            };
            let place = (generation, number);
            if number < current.plan.suitcases.len()
                && self.preparing != Some(place)
                && !self.open.iter().any(|open| open.place == place)
            {
                break number;
            }
        };
        let planned = &current.plan.suitcases[number];
        let need = planned.bytes;
        // Room first: the ones used longest ago leave, never one a reader
        // holds.
        let mut evictable: Vec<usize> = (0..self.open.len())
            .filter(|index| !self.is_held(self.open[*index].place, now))
            .collect();
        evictable.sort_by_key(|index| self.open[*index].last_used);
        let mut free = budget.available();
        let mut leaving = Vec::new();
        for index in evictable.iter().copied() {
            if free >= need {
                break;
            }
            free += self.open[index].bytes;
            leaving.push(index);
        }
        let beyond = need > budget.total();
        let fits = if beyond {
            // A file bigger than the budget: alone, once all else can go.
            leaving.len() == evictable.len() && evictable.len() == self.open.len()
        } else {
            free >= need
        };
        if !fits {
            self.queue.push_front(number);
            return Next::NoRoom;
        }
        leaving.sort_unstable_by(|a, b| b.cmp(a));
        let evict = leaving
            .into_iter()
            .map(|index| {
                let open = self.open.swap_remove(index);
                (open.id, open.bytes)
            })
            .collect();
        let files = current.files[planned.entries.clone()].to_vec();
        self.preparing = Some((generation, number));
        Next::Job(Job {
            place: (generation, number),
            files,
            bytes: need,
            evict,
            beyond,
        })
    }

    /// Takes in a finished job; true if it found no room (try again soon).
    fn done(&mut self, done: Done) -> bool {
        if self.preparing == Some(done.place) {
            self.preparing = None;
        }
        match done.result {
            Ok(Some(prepared)) => {
                if !prepared.changed.is_empty() {
                    tracing::info!(
                        files = prepared.changed.len(),
                        "files changed since they were read; the next pass reads them again"
                    );
                }
                self.open.push(Open {
                    id: prepared.id,
                    place: done.place,
                    bytes: done.bytes,
                    last_used: Instant::now(),
                    ready: prepared.ready,
                });
                false
            }
            Ok(None) => {
                if done.place.0 == self.generation {
                    self.queue.push_front(done.place.1);
                }
                true
            }
            Err(error) => {
                tracing::warn!(%error, "a suitcase could not be prepared");
                false
            }
        }
    }
}

/// Where to note the files that changed.
struct Forget {
    store: Arc<dyn Store>,
    stick_id: StickId,
}

/// Evicts, reserves, prepares. `result` is `Ok(None)` if the room was taken
/// meanwhile.
async fn run_job(
    publisher: &impl Publisher,
    cipher: Arc<dyn Cipher>,
    budget: Budget,
    forget: Forget,
    job: Job,
) -> Done {
    for (id, bytes) in job.evict {
        if let Err(error) = publisher.evict(id).await {
            tracing::warn!(%error, "a suitcase could not be removed");
        }
        budget.release(bytes);
    }
    if job.beyond {
        budget.reserve_beyond(job.bytes);
    } else if !budget.reserve(job.bytes) {
        return Done {
            place: job.place,
            bytes: job.bytes,
            result: Ok(None),
        };
    }
    let result = prepare(publisher, &cipher, job.files).await;
    match &result {
        Err(_) => budget.release(job.bytes),
        // Their record no longer matches them (a change that kept size and
        // times, see "Known limits"): the next pass reads them again.
        Ok(prepared) if !prepared.changed.is_empty() => {
            let Forget { store, stick_id } = forget;
            let paths = prepared.changed.clone();
            let forgotten = run_blocking(move || store.forget_owned_files(&stick_id, &paths)).await;
            if let Err(error) = forgotten {
                tracing::warn!(%error, "changed files could not be marked for reading");
            }
        }
        Ok(_) => {}
    }
    Done {
        place: job.place,
        bytes: job.bytes,
        result: result.map(Some),
    }
}

/// The version being served: its manifest (kept by the last publish) with
/// each file's place on disk and, from the catalogue, its plaintext hash when
/// it belongs to this very blob.
async fn load_current(context: &Context) -> Result<Option<Current>, ShareError> {
    let stick_id = context.stick_id;
    let store = context.store.clone();
    let cache_dir = context.cache_dir.clone();
    let cipher = context.cipher.clone();
    let root = context.root.clone();
    let suitcase_bytes = context.suitcase_bytes;
    run_blocking(move || {
        let owned = store.owned_stick(&stick_id)?.ok_or(ShareError::NotOwned)?;
        let Some(record) = owned.latest_record else {
            return Ok(None);
        };
        let manifest_hash = record.unverified_record().manifest_hash;
        let Some(blob) = load_latest_manifest(&cache_dir, &manifest_hash) else {
            return Ok(None);
        };
        let mut plaintext = Vec::new();
        decrypt_blob(cipher.as_ref(), &mut blob.as_slice(), &mut plaintext)?;
        let manifest = Manifest::from_bytes(&plaintext)?;
        let recorded: HashMap<String, ([u8; 32], Option<[u8; 32]>)> = store
            .owned_files(&stick_id)?
            .into_iter()
            .map(|file| (file.path, (file.blob_hash, file.plaintext_hash)))
            .collect();
        let files = manifest
            .entries
            .iter()
            .map(|entry| FileJob {
                entry: entry.clone(),
                on_disk: on_disk(&root, &entry.path),
                plaintext_hash: recorded
                    .get(&entry.path)
                    .filter(|(blob_hash, _)| *blob_hash == entry.hash)
                    .and_then(|(_, plaintext_hash)| *plaintext_hash),
            })
            .collect();
        let plan = plan(&manifest.entries, cipher.as_ref(), suitcase_bytes);
        Ok(Some(Current {
            manifest_hash,
            plan,
            files,
        }))
    })
    .await
}

/// Where a manifest path is under `root`.
fn on_disk(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

#[cfg(test)]
mod tests;
