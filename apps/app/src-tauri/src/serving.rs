//! Sharing the app's stick while the app runs: the publisher, the watcher
//! that publishes changes, and the reader activity, all in one task that
//! stops cleanly (for "Stop sharing" and Quit).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use ourstick_core::clock::SystemClock;
use ourstick_core::device::{self, ShareLock, StickKeys};
use ourstick_core::secrets::check_secret_store;
use ourstick_core::share::{keep_suitcases, latest_hint, publish_folder, serve_latest, Budget};
use ourstick_core::transport::iroh::{IrohConfig, IrohPublisher, Network};
use ourstick_core::transport::{Publisher, ReaderActivity};
use ourstick_core::watcher::{
    follow_folder, follow_folder_reading_first, FolderEvents, NotifyEvents, Owner, PeriodicEvents,
    WatchEvent, WatchTiming,
};
use ourstick_core::{Manifest, ManifestHint, Store, Ticket};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, watch};

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
}

impl Serving {
    /// Stops serving and waits until the publisher is closed and the share
    /// lock released.
    pub async fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.done.await;
    }
}

/// A problem with sharing, as the UI hears of it (event `share-problem`).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShareProblem {
    /// Could not publish a change; tried again later.
    Failed { message: String },
    /// The stick's folder is not there; the last version is still served.
    Ejected,
    /// Publishing works again.
    Cleared,
}

/// Who downloads (event `activity`).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ActivityView {
    pub readers: usize,
    pub bytes_per_second: u64,
    /// What to do when readers reach this computer only through the relay
    /// (see [`relay_only_message`]); `None` otherwise.
    pub relay_only: Option<&'static str>,
}

impl From<ReaderActivity> for ActivityView {
    fn from(activity: ReaderActivity) -> Self {
        Self {
            readers: activity.readers,
            bytes_per_second: activity.bytes_per_second,
            relay_only: activity.only_relayed.then(relay_only_message),
        }
    }
}

/// Readers reach this computer only through the relay: most likely its
/// firewall drops what they send (on Windows, after "Cancel" on the
/// Windows Firewall question). Says so, and how to fix it here.
pub fn relay_only_message() -> &'static str {
    if cfg!(windows) {
        "People can download only through the relay, which is slower. Windows \
         Firewall probably blocks OurStick: open Windows Security, Firewall & \
         network protection, Allow an app through firewall, and tick OurStick \
         for the network you are on."
    } else if cfg!(target_os = "macos") {
        "People can download only through the relay, which is slower. The \
         firewall probably blocks OurStick: open System Settings, Network, \
         Firewall, Options, and allow incoming connections for OurStick."
    } else {
        "People can download only through the relay, which is slower. A \
         firewall probably drops what they send: allow incoming UDP for \
         OurStick (e.g. in ufw or firewalld)."
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Starts sharing the drive stick. Takes the device's share lock first: if the
/// CLI already shares on this device, this fails and says so.
pub async fn start(app: AppHandle, data_dir: PathBuf, store: Arc<dyn Store>) -> Result<Serving> {
    let (lock, node_key, keys) = tauri::async_runtime::spawn_blocking({
        let data_dir = data_dir.clone();
        let store = store.clone();
        move || -> Result<(ShareLock, [u8; 32], StickKeys)> {
            let lock = ShareLock::acquire(&data_dir)?;
            let secrets = device::open_system_keyring()?;
            check_secret_store(secrets.as_ref())?;
            let backend = device::system_keyring_backend().context("no keyring on this system")?;
            let node_key =
                device::open_device(store.as_ref(), &data_dir, secrets.as_ref(), backend)?;
            let stick = store
                .owned_sticks()?
                .into_iter()
                .find(|stick| stick.drive_size.is_some())
                .context("there is no stick yet")?;
            Ok((lock, node_key, device::load_stick(secrets.as_ref(), stick)?))
        }
    })
    .await??;

    let stick_id = keys.stick.stick_id;
    let cache_dir = data_dir.join("cache").join(hex(&stick_id.to_bytes()));
    let config = IrohConfig::new(node_key, Network::Public);
    let publisher = IrohPublisher::start(&config, stick_id, &cache_dir).await?;
    // ADR 0007: files are encrypted into suitcases as readers ask for them,
    // within a budget. With too little free space, sharing does not start
    // and the error says why.
    let budget = {
        let cache_dir = cache_dir.clone();
        tauri::async_runtime::spawn_blocking(move || Budget::for_cache(&cache_dir)).await??
    };
    tauri::async_runtime::spawn({
        let keeper = keep_suitcases(
            publisher.clone(),
            store.clone(),
            stick_id,
            keys.stick_key.clone(),
            budget,
        );
        async move {
            if let Err(error) = keeper.await {
                tracing::warn!(%error, "the suitcases' keeper stopped");
            }
        }
    });
    // The last version at once, if the cache still holds it; the folder is
    // then read again in the background (`Shared::run`). Otherwise (a new
    // stick, a cache that is gone) a first pass now, for the link.
    let latest = serve_latest(&Owner {
        publisher: &publisher,
        store: store.clone(),
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
                &publisher,
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
            on_watch_event(&app, &WatchEvent::Failed { error });
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
    let activity = publisher.activity();

    let (stop, stopped) = oneshot::channel();
    let root = keys.stick.root_path.clone();
    let shared = Shared {
        app,
        publisher,
        keys,
        store,
        root: root.clone(),
        current,
        read_again,
        lock,
    };
    let done = match NotifyEvents::start(&root) {
        Ok(events) => tauri::async_runtime::spawn(shared.run(events, stopped)),
        Err(error) => {
            tracing::warn!(%error, "the folder cannot be watched; checking it now and then");
            let events = PeriodicEvents::new(RESCAN_EVERY);
            tauri::async_runtime::spawn(shared.run(events, stopped))
        }
    };
    Ok(Serving {
        stop,
        done,
        link,
        activity,
    })
}

/// What the sharing task owns.
struct Shared {
    app: AppHandle,
    publisher: IrohPublisher,
    keys: StickKeys,
    store: Arc<dyn Store>,
    root: PathBuf,
    current: Manifest,
    /// The folder was not read at start: a pass comes first.
    read_again: bool,
    lock: ShareLock,
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
            root,
            current,
            read_again,
            lock,
        } = self;
        {
            let owner = Owner {
                publisher: &publisher,
                store,
                clock: &SystemClock,
                signing_key: &keys.signing_key,
                stick_key: &keys.stick_key,
            };
            let mut on_event = |event: WatchEvent<'_>| on_watch_event(&app, &event);
            let mut activity = publisher.activity();
            let forward = async {
                let mut awake = crate::inhibit::KeepAwake::default();
                while activity.changed().await.is_ok() {
                    let now = *activity.borrow();
                    crate::tray::set_active(&app, now.readers > 0);
                    awake.set(now.readers > 0).await;
                    let _ = app.emit("activity", ActivityView::from(now));
                }
            };
            let timing = WatchTiming::default();
            let follow = async {
                if read_again {
                    follow_folder_reading_first(
                        &owner,
                        &root,
                        &mut events,
                        timing,
                        current,
                        &mut on_event,
                    )
                    .await
                } else {
                    follow_folder(&owner, &root, &mut events, timing, current, &mut on_event).await
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
fn on_watch_event(app: &AppHandle, event: &WatchEvent<'_>) {
    let problem = match event {
        WatchEvent::Published { .. } | WatchEvent::Unchanged => {
            let _ = app.emit("stick-changed", ());
            ShareProblem::Cleared
        }
        WatchEvent::StillChanging | WatchEvent::Reading { .. } => return,
        WatchEvent::Failed { error } => ShareProblem::Failed {
            message: error.to_string(),
        },
        WatchEvent::Ejected => ShareProblem::Ejected,
    };
    let _ = app.emit("share-problem", problem);
}
