//! The sticks this device receives in the app (step 7b): each kept up to
//! date in the background while the app runs, its download lock held from
//! the moment it is added until it is forgotten (paused too), so `ourstick
//! get` never touches it. What each one is doing, for the window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ourstick_core::clock::SystemClock;
use ourstick_core::device::random_key;
use ourstick_core::download::{
    receive, Cancel, ReceiveEvent, Receiving, RelayPolicy, FOLLOW_EVERY,
};
use ourstick_core::locks::Lock;
use ourstick_core::received::received_store_dir;
use ourstick_core::secrets::SecretName;
use ourstick_core::space::DownloadRoom;
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, Network};
use ourstick_core::transport::Fetcher;
use ourstick_core::{StickId, Ticket};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

pub mod commands;
pub mod preview;
mod told;
mod words;

use told::Told;

pub use words::{stopped_words, waiting_words};

/// What a received stick is doing, as the window shows it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Shown {
    /// "starting", "downloading", "getting_more", "up_to_date", "waiting",
    /// "paused" or "stopped".
    pub state: &'static str,
    /// Why it waits or stopped, in words for everyone.
    pub message: Option<String>,
    /// Bytes landed of those planned in this round.
    pub done: u64,
    pub total: u64,
    /// When it began waiting (ms since the epoch), for "since 14:30".
    pub since_ms: Option<u64>,
}

/// One received stick the app holds.
struct Held {
    /// Its download lock: no other program receives it meanwhile.
    lock: Lock,
    /// Its receive, while one runs.
    running: Option<(Arc<Cancel>, tauri::async_runtime::JoinHandle<()>)>,
}

/// The app's received sticks, by id.
#[derive(Default)]
pub struct Receivers {
    held: tokio::sync::Mutex<HashMap<StickId, Held>>,
    shown: Mutex<HashMap<StickId, Shown>>,
}

impl Receivers {
    pub fn shown(&self, stick_id: &StickId) -> Shown {
        lock(&self.shown).get(stick_id).cloned().unwrap_or_default()
    }

    fn show(&self, stick_id: StickId, change: impl FnOnce(&mut Shown)) {
        change(lock(&self.shown).entry(stick_id).or_default());
    }

    /// The received sticks downloading now, for the Quit dialog.
    pub fn downloading(&self) -> Vec<StickId> {
        lock(&self.shown)
            .iter()
            .filter(|(_, shown)| matches!(shown.state, "downloading" | "getting_more"))
            .map(|(stick_id, _)| *stick_id)
            .collect()
    }

    /// Holds `stick_id` with its `lock` (not running yet).
    pub async fn hold(&self, stick_id: StickId, lock: Lock) {
        self.held.lock().await.insert(
            stick_id,
            Held {
                lock,
                running: None,
            },
        );
    }

    pub async fn is_held(&self, stick_id: &StickId) -> bool {
        self.held.lock().await.contains_key(stick_id)
    }

    /// Stops `stick_id`'s receive, if one runs, and waits until it ended.
    pub async fn stop(&self, stick_id: &StickId) {
        let running = match self.held.lock().await.get_mut(stick_id) {
            Some(held) => held.running.take(),
            None => None,
        };
        if let Some((cancel, task)) = running {
            cancel.cancel();
            let _ = task.await;
        }
    }

    /// Stops and lets go of `stick_id`, returning its lock (forget).
    pub async fn release(&self, stick_id: &StickId) -> Option<Lock> {
        self.stop(stick_id).await;
        lock(&self.shown).remove(stick_id);
        self.held
            .lock()
            .await
            .remove(stick_id)
            .map(|held| held.lock)
    }

    /// Stops every receive (Quit): each keeps what it received, and goes on
    /// where it was next time. At most a few seconds.
    pub async fn stop_all(&self) {
        let running: Vec<_> = self
            .held
            .lock()
            .await
            .values_mut()
            .filter_map(|held| held.running.take())
            .collect();
        for (cancel, _) in &running {
            cancel.cancel();
        }
        for (_, task) in running {
            // A round may take its grace (10 s) to finish the file it places.
            let _ = tokio::time::timeout(Duration::from_secs(12), task).await;
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Tells the window the received sticks changed.
fn changed(app: &AppHandle) {
    let _ = app.emit("received-changed", ());
}

/// Starts receiving `stick_id` (held already) into `folder` with `ticket`,
/// in the background.
pub async fn start(app: &AppHandle, stick_id: StickId, ticket: Ticket, folder: std::path::PathBuf) {
    let receivers = app.state::<Receivers>();
    let cancel = Cancel::new();
    receivers.show(stick_id, |shown| {
        *shown = Shown {
            state: "starting",
            ..Shown::default()
        }
    });
    changed(app);
    let task = {
        let app = app.clone();
        let cancel = cancel.clone();
        tauri::async_runtime::spawn(async move {
            run(&app, stick_id, ticket, folder, &cancel).await;
        })
    };
    let mut held = receivers.held.lock().await;
    match held.get_mut(&stick_id) {
        Some(held) => held.running = Some((cancel, task)),
        // Forgotten meanwhile: not wanted.
        None => cancel.cancel(),
    }
}

/// One stick's receive, until cancelled or stopped for the user.
async fn run(
    app: &AppHandle,
    stick_id: StickId,
    ticket: Ticket,
    folder: std::path::PathBuf,
    cancel: &Cancel,
) {
    let state = app.state::<AppState>();
    let store_dir = received_store_dir(&state.data_dir, &stick_id);
    // A reader needs no lasting identity: a fresh node key per run.
    let config = IrohConfig::new(random_key(), Network::Public);
    let fetcher = match IrohFetcher::start(&config, &store_dir).await {
        Ok(fetcher) => fetcher,
        Err(error) => return stopped(app, stick_id, format!("cannot start networking: {error}")),
    };
    let room = DownloadRoom::new(
        state.store.clone(),
        state.data_dir.clone(),
        stick_id,
        folder.clone(),
    );
    let policy = RelayPolicy::default();
    let receiving = Receiving {
        kind: ourstick_core::StickKind::App,
        fetcher: &fetcher,
        store: state.store.clone(),
        clock: &SystemClock,
        ticket: &ticket,
        folder: &folder,
        policy: &policy,
        room: &room,
        follow_every: FOLLOW_EVERY,
    };
    let (downloading, mut awake_wanted) = tokio::sync::watch::channel(false);
    let mut told = Told::new(app.clone(), stick_id, downloading);
    let awake = async {
        let mut keep = crate::inhibit::KeepAwake::default();
        while awake_wanted.changed().await.is_ok() {
            let wanted = *awake_wanted.borrow();
            keep.set(wanted).await;
        }
    };
    let mut on_event = |event: ReceiveEvent<'_>| told.on(event);
    let received = tokio::select! {
        received = receive(&receiving, cancel, &mut on_event) => received,
        () = awake => Ok(()),
    };
    if let Err(error) = received {
        stopped(app, stick_id, stopped_words(&error));
    }
    // Its store stays: the next receive goes on where this one was.
    if let Err(error) = fetcher.shutdown(true).await {
        tracing::warn!(%error, "a received stick's blobs were not closed cleanly");
    }
}

fn stopped(app: &AppHandle, stick_id: StickId, message: String) {
    app.state::<Receivers>().show(stick_id, |shown| {
        *shown = Shown {
            state: "stopped",
            message: Some(message),
            ..Shown::default()
        }
    });
    changed(app);
}

/// A received stick's ticket, kept in the keyring as its sharing link.
pub fn read_ticket(
    secrets: &dyn ourstick_core::SecretStore,
    stick_id: &StickId,
) -> anyhow::Result<Ticket> {
    use anyhow::Context;
    let stored = secrets
        .get(&SecretName::Ticket(*stick_id))?
        .context("its sharing link is not in the keyring any more; add it again")?;
    let text = String::from_utf8(stored).context("its stored sharing link is damaged")?;
    // The error never repeats the link: it holds the stick's key.
    text.parse()
        .map_err(|_| anyhow::anyhow!("its stored sharing link is damaged; add it again"))
}

/// Keeps `ticket` in the keyring as the stick's sharing link.
pub fn keep_ticket(
    secrets: &dyn ourstick_core::SecretStore,
    ticket: &Ticket,
) -> anyhow::Result<()> {
    let link = ticket.to_link()?;
    secrets.put(&SecretName::Ticket(ticket.stick_id), link.as_bytes())?;
    Ok(())
}

/// At start: every stick the app receives held, and those not paused
/// received again. Blocking parts off the async runtime.
pub async fn start_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let found = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Vec<_>> {
        let secrets = ourstick_core::device::open_system_keyring()?;
        let mut found = Vec::new();
        for joined in store.joined_sticks()? {
            if joined.kind != ourstick_core::StickKind::App {
                continue;
            }
            let lock = ourstick_core::locks::download_lock(&data_dir, &joined.stick_id);
            let ticket = read_ticket(secrets.as_ref(), &joined.stick_id);
            found.push((joined, lock, ticket));
        }
        Ok(found)
    })
    .await;
    let found = match found {
        Ok(Ok(found)) => found,
        Ok(Err(error)) => {
            return crate::problems::failed(app, None, format!("Received sticks: {error:#}"))
        }
        Err(error) => return tracing::warn!(%error, "received sticks were not read"),
    };
    let receivers = app.state::<Receivers>();
    for (joined, lock, ticket) in found {
        let stick_id = joined.stick_id;
        let lock = match lock {
            Ok(lock) => lock,
            Err(error) => {
                stopped(app, stick_id, format!("{error}"));
                continue;
            }
        };
        receivers.hold(stick_id, lock).await;
        match ticket {
            _ if joined.paused => receivers.show(stick_id, |shown| {
                *shown = Shown {
                    state: "paused",
                    ..Shown::default()
                }
            }),
            Ok(ticket) => start(app, stick_id, ticket, joined.output_dir).await,
            Err(error) => stopped(app, stick_id, format!("{error:#}")),
        }
    }
    changed(app);
}

#[cfg(test)]
mod tests;
