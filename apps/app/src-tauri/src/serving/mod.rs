//! Sharing the app's stick while the app runs: the publisher, the watcher
//! that publishes changes, and the reader activity, all in one task that
//! stops cleanly (for "Stop sharing" and Quit).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use ourstick_core::clock::SystemClock;
use ourstick_core::device::{self, StickKeys};
use ourstick_core::locks::{self, SharingLocks};
use ourstick_core::secrets::check_secret_store;
use ourstick_core::share::{keep_files, latest_hint, publish_folder, serve_latest};
use ourstick_core::transport::iroh::{IrohConfig, IrohPublisher, Network};
use ourstick_core::transport::{Publisher, ReaderActivity};
use ourstick_core::watcher::{
    follow_folder, follow_folder_reading_first, FolderEvents, NotifyEvents, Nudge, Owner,
    PeriodicEvents, WatchEvent, WatchTiming,
};
use ourstick_core::{Manifest, ManifestHint, StickId, Store, Ticket};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{oneshot, watch};

mod events;
pub mod readers;

pub use events::{ActivityView, ShareProblem, StickEvent};

/// How often the folder is checked when the system cannot report changes.
const RESCAN_EVERY: Duration = Duration::from_secs(60);

/// A running share, until it is stopped.
pub struct Serving {
    stop: oneshot::Sender<()>,
    done: tauri::async_runtime::JoinHandle<()>,
    /// The link, once the stick has been published (never logged).
    pub link: Option<String>,
    /// Who downloads now.
    pub activity: watch::Receiver<ReaderActivity>,
    /// Tells the watcher to look now (a linked entry came or went).
    pub nudge: Nudge,
}

impl Serving {
    /// Stops serving and waits until the publisher is closed and the share
    /// lock released.
    pub async fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.done.await;
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Starts sharing the app's stick `stick_id`. Takes its lock first (ADR 0009): if
/// another program shares it, or shares another older stick while this one
/// is older too, this fails and says so. The device was recorded at the
/// app's start (`sharing::start_app`).
pub async fn start(
    app: AppHandle,
    data_dir: PathBuf,
    store: Arc<dyn Store>,
    stick_id: StickId,
) -> Result<Serving> {
    let (lock, keys) = tauri::async_runtime::spawn_blocking({
        let data_dir = data_dir.clone();
        let store = store.clone();
        move || -> Result<(SharingLocks, StickKeys)> {
            let secrets = device::open_system_keyring()?;
            check_secret_store(secrets.as_ref())?;
            let (lock, stick) = locks::lock_for_sharing(&data_dir, store.as_ref(), &stick_id)?;
            let keys = device::load_stick(store.as_ref(), secrets.as_ref(), stick)?;
            Ok((lock, keys))
        }
    })
    .await??;

    let stick_id = keys.stick.stick_id;
    let cache_dir = data_dir.join("cache").join(hex(&stick_id.to_bytes()));
    let config = IrohConfig::new(keys.node_key, Network::Public);
    let publisher = IrohPublisher::start(&config, stick_id, &cache_dir).await?;
    // Who has it (ADR 0015): readers' reports kept from the start.
    if let Some(reports) = publisher.take_reports() {
        readers::keep(
            &app,
            reports,
            store.clone(),
            stick_id,
            keys.stick_key.clone(),
        );
    }
    // Whatever fails from here stops the endpoint before the locks go: no
    // second start may bind this node key while it still runs.
    let (current, link, read_again) = match first_publication(&app, &publisher, &store, &keys).await
    {
        Ok(first) => first,
        Err(error) => {
            publisher.clone().shutdown().await;
            drop(lock);
            return Err(error);
        }
    };
    let activity = publisher.activity();

    let (stop, stopped) = oneshot::channel();
    let root = keys.stick.root_path.clone();
    let shared = Shared {
        app,
        publisher,
        keys,
        store,
        current,
        read_again,
        lock,
    };
    let (done, nudge) = match NotifyEvents::start(&root) {
        Ok(events) => {
            let nudge = events.nudge();
            (
                tauri::async_runtime::spawn(shared.run(events, stopped)),
                nudge,
            )
        }
        Err(error) => {
            tracing::warn!(%error, "the folder cannot be watched; checking it now and then");
            let events = PeriodicEvents::new(RESCAN_EVERY);
            let nudge = events.nudge();
            (
                tauri::async_runtime::spawn(shared.run(events, stopped)),
                nudge,
            )
        }
    };
    Ok(Serving {
        stop,
        done,
        link,
        activity,
        nudge,
    })
}

/// The keeper of the owner's files (ADR 0017), then the first version (from
/// the cache, or a pass) and the link: what the current manifest is, the
/// link, and whether the folder must be read again.
async fn first_publication(
    app: &AppHandle,
    publisher: &IrohPublisher,
    store: &Arc<dyn Store>,
    keys: &StickKeys,
) -> Result<(Manifest, Option<String>, bool)> {
    let stick_id = keys.stick.stick_id;
    let disks = app.state::<crate::state::AppState>().disks.clone();
    tauri::async_runtime::spawn({
        let keeper = keep_files(
            publisher.clone(),
            store.clone(),
            stick_id,
            keys.stick_key.clone(),
            disks.clone(),
        );
        async move {
            if let Err(error) = keeper.await {
                tracing::warn!(%error, "the files' keeper stopped");
            }
        }
    });
    // The last version at once, if the cache still holds it; the folder is
    // then read again in the background (`Shared::run`). Otherwise (a new
    // stick, a cache that is gone) a first pass now, for the link.
    let latest = serve_latest(&Owner {
        publisher,
        store: store.clone(),
        disks,
        clock: &SystemClock,
        signing_key: &keys.signing_key,
        stick_key: &keys.stick_key,
    })
    .await;
    let read_again = matches!(latest, Ok(Some(_)));
    let first = match latest {
        Ok(Some(publication)) => Ok(publication),
        Ok(None) | Err(_) => {
            publish_folder(
                publisher,
                store.clone(),
                &SystemClock,
                &keys.signing_key,
                &keys.stick_key,
                false,
            )
            .await
        }
    };
    let (current, hint) = match &first {
        Ok(publication) => (
            publication.manifest.clone(),
            Some(ManifestHint {
                seq: publication.seq,
                hash: publication.manifest_hash,
            }),
        ),
        Err(error) => {
            on_watch_event(app, &stick_id, &WatchEvent::Failed { error });
            // The link can still name a version published in an earlier run.
            let hint = {
                let store = store.clone();
                tauri::async_runtime::spawn_blocking(move || latest_hint(store.as_ref(), &stick_id))
                    .await??
            };
            (Manifest::empty(), hint)
        }
    };
    // Wait for the home relay, so the link carries it.
    publisher.online(Duration::from_secs(30)).await;
    let link = match hint {
        Some(manifest_hint) => Some(
            Ticket {
                stick_id,
                node_id: publisher.node_id(),
                manifest_hint,
                address_hints: publisher.address_hints(),
                stick_key: keys.stick_key.clone(),
            }
            .to_link()?,
        ),
        None => None,
    };
    Ok((current, link, read_again))
}

/// What the sharing task owns.
struct Shared {
    app: AppHandle,
    publisher: IrohPublisher,
    keys: StickKeys,
    store: Arc<dyn Store>,
    current: Manifest,
    /// The folder was not read at start: a pass comes first.
    read_again: bool,
    lock: SharingLocks,
}

impl Shared {
    /// Follows the folder and forwards the activity until told to stop, then
    /// closes the publisher and releases the lock.
    async fn run(self, mut events: impl FolderEvents, stopped: oneshot::Receiver<()>) {
        let Shared {
            app,
            publisher,
            keys,
            store,
            current,
            read_again,
            lock,
        } = self;
        {
            let owner = Owner {
                publisher: &publisher,
                store,
                disks: app.state::<crate::state::AppState>().disks.clone(),
                clock: &SystemClock,
                signing_key: &keys.signing_key,
                stick_key: &keys.stick_key,
            };
            let stick_id = keys.stick.stick_id;
            let mut on_event = |event: WatchEvent<'_>| on_watch_event(&app, &stick_id, &event);
            let mut activity = publisher.activity();
            let forward = async {
                let mut awake = crate::inhibit::KeepAwake::default();
                while activity.changed().await.is_ok() {
                    let now = *activity.borrow();
                    awake.set(now.readers > 0).await;
                    let _ = app.emit(
                        "activity",
                        StickEvent::new(&stick_id, ActivityView::from(now)),
                    );
                    // The green dot: anyone downloading from any stick.
                    crate::sharing::refresh_active(&app).await;
                }
            };
            let timing = WatchTiming::default();
            let follow = async {
                if read_again {
                    follow_folder_reading_first(&owner, &mut events, timing, current, &mut on_event)
                        .await
                } else {
                    follow_folder(&owner, &mut events, timing, current, &mut on_event).await
                }
            };
            tokio::select! {
                () = follow => {}
                () = forward => {}
                _ = stopped => {}
            }
        }
        publisher.shutdown().await;
        drop(lock);
    }
}

/// Tells the UI what the watcher did.
fn on_watch_event(app: &AppHandle, stick_id: &StickId, event: &WatchEvent<'_>) {
    let problem = match event {
        WatchEvent::Published { .. } | WatchEvent::Unchanged => {
            let _ = app.emit("stick-changed", ());
            ShareProblem::Cleared
        }
        WatchEvent::StillChanging | WatchEvent::Reading { .. } => return,
        WatchEvent::SeenSlowly => ShareProblem::SeenSlowly,
        // The window lists linked entries that are gone and own entries
        // left out from the stick itself; it only needs to look again.
        WatchEvent::Roots { .. } => {
            let _ = app.emit("stick-changed", ());
            return;
        }
        WatchEvent::Failed { error } => ShareProblem::Failed {
            message: crate::words::told_one(*error),
        },
        WatchEvent::Ejected => ShareProblem::Ejected,
        // Not a problem: a note while it lasts, told at each change.
        WatchEvent::GivingWay(now) => {
            let view = events::GivingWayView { giving_way: *now };
            let _ = app.emit("giving-way", StickEvent::new(stick_id, view));
            return;
        }
    };
    crate::problems::tell(app, Some(*stick_id), problem);
}
