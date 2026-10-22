//! The owner's side of ADR 0017: for each version the owner publishes, the
//! files its own store serves; and, one at a time through the disks' turns
//! (ADR 0013), the files it cannot serve yet made ready (rule 7): their
//! plaintext hash and outboard, first those readers wait on, then the rest
//! in manifest order. It replaced ADR 0007's suitcases.

mod keeper;
mod make;
mod version;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use self::keeper::Keeper;
use self::make::{make, Made, ToMake};
use super::ShareError;
use crate::blocking::run_blocking;
use crate::cipher::{cipher_for, Cipher, StickKey};
use crate::disk::turns::DiskTurns;
use crate::layout::StickLayout;
use crate::manifest::hashing::outboards_dir;
use crate::stick_id::StickId;
use crate::store::Store;
use crate::transport::{BlobHash, Publisher};

/// How often the keeper tries again the files that could not be read, and
/// a version whose records could not be read; it does not look while there
/// are none.
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
    disks: Arc<DiskTurns>,
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
        outboards: outboards_dir(publisher.cache_dir()),
        disks,
        pass_wanted,
        stop: Arc::new(AtomicBool::new(false)),
    };
    // Each version as the owner publishes it, decoded once (ADR 0019).
    let mut versions = publisher.published();
    let mut keeper = Keeper::default();
    let first = versions.borrow_and_update().clone();
    keeper.reload(first, &publisher, &context).await;
    // A blob being made, and the version it was made for.
    let mut job: Option<(Option<BlobHash>, Making)> = None;
    let mut look = tokio::time::interval(LOOK_EVERY);
    look.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let ended = loop {
        if job.is_none() {
            if let (Some(blob), Some(layout)) = (keeper.next(), keeper.layout.clone()) {
                let urgent = publisher.readers_waiting(READERS_WAIT_WINDOW);
                let making = start_making(&context, blob, layout, urgent);
                job = Some((keeper.manifest_hash, making));
            }
        }
        tokio::select! {
            event = demand.recv() => match event {
                Some(event) => keeper.on_demand(event, &context).await,
                None => break Ok(()),
            },
            done = async { (&mut job.as_mut().expect("guarded").1).await }, if job.is_some() => {
                let made_for = job.take().expect("guarded").0;
                match done {
                    // Of a version since replaced: its paths may be others
                    // now, and the blob, if still listed, is made again.
                    Ok(_) if made_for != keeper.manifest_hash => {}
                    Ok((blob, made)) => keeper.done(blob, made, &publisher, &context).await,
                    Err(error) => tracing::warn!(%error, "making a file ready failed"),
                }
            },
            changed = versions.changed() => {
                if changed.is_err() {
                    // Never while this keeper holds its publisher (and the
                    // sender with it); ended rather than spun on.
                    break Ok(());
                }
                let version = versions.borrow_and_update().clone();
                keeper.reload(version, &publisher, &context).await;
            },
            _ = look.tick(), if keeper.has_to_look() => {
                keeper.retry_due();
                let version = versions.borrow().clone();
                keeper.reload(version, &publisher, &context).await;
            }
        }
    };
    // A file being made stops at its next disk window.
    context.stop.store(true, Ordering::Relaxed);
    ended
}

/// A blob being made, and what came of it.
type Making = tokio::task::JoinHandle<(ToMake, Made)>;

/// What the keeper works with.
struct Context {
    store: Arc<dyn Store>,
    stick_id: StickId,
    cipher: Arc<dyn Cipher>,
    outboards: PathBuf,
    disks: Arc<DiskTurns>,
    pass_wanted: PassWanted,
    stop: Arc<AtomicBool>,
}

fn start_making(context: &Context, blob: ToMake, layout: StickLayout, urgent: bool) -> Making {
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
