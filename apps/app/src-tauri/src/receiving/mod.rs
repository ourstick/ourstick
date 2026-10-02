//! The sticks this device receives in the app (step 7b): each kept up to
//! date in the background while the app runs, its download lock held from
//! the moment it is added until it is forgotten (paused too), so `ourstick
//! get` never touches it. What each one is doing, for the window.

use std::sync::Mutex;

use ourstick_core::clock::SystemClock;
use ourstick_core::device::random_key;
use ourstick_core::download::{
    receive, Cancel, ReceiveEvent, Receiving, RelayPolicy, FOLLOW_EVERY,
};
use ourstick_core::locks::{download_lock, take_retrying, Lock, LockError};
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
mod holder;
pub mod preview;
mod told;
mod words;

pub use holder::Receivers;
use told::Told;

pub use words::{said, stopped_words, waiting_words};

/// `work` off the async runtime; its error in words for everyone.
pub(crate) async fn work<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, String> {
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(said(&error)),
        Err(error) => Err(said(&anyhow::Error::from(error))),
    }
}

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

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Tells the window the received sticks changed.
fn changed(app: &AppHandle) {
    let _ = app.emit("received-changed", ());
}

/// Why this program could not take a received stick's lock, in words.
pub(super) fn lock_words(error: &LockError) -> String {
    match error {
        LockError::DownloadInUse => "Another OurStick program on this computer is getting this \
             stick right now. Close it, then use Try again."
            .to_owned(),
        other => {
            tracing::warn!(error = %other, "a received stick's lock");
            "OurStick could not reserve this stick on this computer. Use Try again.".to_owned()
        }
    }
}

/// Takes `stick_id`'s download lock, once more after a moment if another
/// program is only looking at it. Off the async runtime.
async fn take_lock(data_dir: std::path::PathBuf, stick_id: StickId) -> Result<Lock, String> {
    let taken = tauri::async_runtime::spawn_blocking(move || {
        take_retrying(|| download_lock(&data_dir, &stick_id))
    })
    .await;
    match taken {
        Ok(Ok(lock)) => Ok(lock),
        Ok(Err(error)) => Err(lock_words(&error)),
        Err(error) => Err(error.to_string()),
    }
}

/// Starts (again) receiving the held stick `stick_id` into `folder` with
/// `ticket`, in the background: a receive of it that runs stops first, and
/// its lock is taken if this program does not have it yet (else it stops,
/// saying who holds it). With `resume`, its pause is lifted under the same
/// mutex (a Pause never lands between). Nothing if it is not held
/// (forgotten meanwhile) or the app is quitting.
pub async fn start(
    app: &AppHandle,
    stick_id: StickId,
    ticket: Ticket,
    folder: std::path::PathBuf,
    resume: bool,
) {
    let receivers = app.state::<Receivers>();
    let state = app.state::<AppState>();
    let mut held = receivers.held.lock().await;
    if receivers.quitting.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let Some(entry) = held.get_mut(&stick_id) else {
        return;
    };
    entry.stop().await;
    if resume {
        let store = state.store.clone();
        let lifted = work(move || {
            Ok(store.change_joined_stick(
                &stick_id,
                &ourstick_core::store::JoinedChange::Paused(false),
            )?)
        })
        .await;
        if let Err(why) = lifted {
            return stopped(app, stick_id, why);
        }
    }
    let data_dir = state.data_dir.clone();
    if entry.lock.is_none() {
        match take_lock(data_dir, stick_id).await {
            Ok(lock) => entry.lock = Some(lock),
            Err(why) => return stopped(app, stick_id, why),
        }
    }
    receivers.show(stick_id, |shown| {
        *shown = Shown {
            state: "starting",
            ..Shown::default()
        }
    });
    changed(app);
    let cancel = Cancel::new();
    let task = {
        let app = app.clone();
        let cancel = cancel.clone();
        tauri::async_runtime::spawn(async move {
            run(&app, stick_id, ticket, folder, &cancel).await;
        })
    };
    entry.running = Some((cancel, task));
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
        Err(error) => {
            tracing::warn!(%error, "a received stick's networking");
            return stopped(
                app,
                stick_id,
                "This computer's network could not be started for it. Use Try again.".to_owned(),
            );
        }
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
    let stored = secrets.get(&SecretName::Ticket(*stick_id))?.context(
        "its sharing link is no longer saved on this computer; forget it and add it again",
    )?;
    let text = String::from_utf8(stored)
        .context("its saved sharing link is damaged; forget it and add it again")?;
    // The error never repeats the link: it holds the stick's key.
    text.parse().map_err(|_| {
        anyhow::anyhow!("its saved sharing link is damaged; forget it and add it again")
    })
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
        let received: Vec<_> = store
            .joined_sticks()?
            .into_iter()
            .filter(|joined| joined.kind == ourstick_core::StickKind::App)
            .collect();
        // The keyring only for sticks it receives.
        if received.is_empty() {
            return Ok(Vec::new());
        }
        let secrets = ourstick_core::device::open_system_keyring()?;
        let mut found = Vec::new();
        for joined in received {
            let lock = take_retrying(|| download_lock(&data_dir, &joined.stick_id));
            let ticket = read_ticket(secrets.as_ref(), &joined.stick_id);
            found.push((joined, lock, ticket));
        }
        Ok(found)
    })
    .await;
    let found = match found {
        Ok(Ok(found)) => found,
        Ok(Err(error)) => {
            return crate::problems::failed(
                app,
                None,
                format!("The sticks you receive: {}", words::said(&error)),
            )
        }
        Err(error) => return tracing::warn!(%error, "received sticks were not read"),
    };
    let receivers = app.state::<Receivers>();
    for (joined, lock, ticket) in found {
        let stick_id = joined.stick_id;
        // Held either way; a lock another program has is taken again when
        // it is started (or with Try again).
        let lock = match lock {
            Ok(lock) => Some(lock),
            Err(error) => {
                stopped(app, stick_id, lock_words(&error));
                None
            }
        };
        let without_lock = lock.is_none();
        receivers.hold(stick_id, lock).await;
        if without_lock {
            continue;
        }
        match ticket {
            _ if joined.paused => receivers.show(stick_id, |shown| {
                *shown = Shown {
                    state: "paused",
                    ..Shown::default()
                }
            }),
            Ok(ticket) => start(app, stick_id, ticket, joined.output_dir, false).await,
            Err(error) => stopped(app, stick_id, format!("{error:#}")),
        }
    }
    changed(app);
}

#[cfg(test)]
mod tests;
