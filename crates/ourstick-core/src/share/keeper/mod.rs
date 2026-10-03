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
mod state;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use budget::{Budget, MAX_BUDGET, MIN_BUDGET};

use self::plan::{plan, Plan, SUITCASE_BYTES};
use self::prepare::{prepare, FileJob, Prepared};
use self::state::State;
use super::disk_turns::Disks;
use super::latest::load_latest_manifest;
use super::ShareError;
use crate::blob::decrypt_blob;
use crate::blocking::run_blocking;
use crate::cipher::{cipher_for, Cipher, StickKey};
use crate::layout::StickLayout;
use crate::manifest::Manifest;
use crate::stick_id::StickId;
use crate::store::Store;
use crate::transport::{BlobHash, Publisher, SuitcaseId};

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
    disks: Arc<Disks>,
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
                        disks.clone(),
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
    disks: Arc<Disks>,
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
    let result = prepare(publisher, &cipher, &disks, job.files).await;
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
    let suitcase_bytes = context.suitcase_bytes;
    run_blocking(move || {
        let owned = store.owned_stick(&stick_id)?.ok_or(ShareError::NotOwned)?;
        // Read again with each version: linked entries come and go.
        let layout = StickLayout::load(store.as_ref(), &stick_id)?;
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
                on_disk: layout.on_disk(&entry.path),
                root: layout.root_of(&entry.path).clone(),
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

#[cfg(test)]
mod tests;
