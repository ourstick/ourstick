//! Links the app waits for (ADR 0025): pasted while their sender was away,
//! held like a received stick (its lock, its task, the one `held` mutex),
//! and made a received stick, everything ticked, once the sender answers.
//! A wait ends after 30 days.

use std::path::PathBuf;
use std::time::Duration;

use ourstick_core::clock::{Clock, SystemClock};
use ourstick_core::device::random_key;
use ourstick_core::download::{wait_for_owner, Cancel, DownloadError, Waiting, FOLLOW_EVERY};
use ourstick_core::locks::{download_lock, take_retrying, LockError};
use ourstick_core::received::check_link;
use ourstick_core::secrets::SecretName;
use ourstick_core::store::WaitingLink;
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, Network};
use ourstick_core::transport::Fetcher;
use ourstick_core::{StickId, StickKind, Ticket};
use tauri::{AppHandle, Manager, State};

use super::commands::ticket_of;
use super::{
    changed, keep_ticket, stopped_words_then, waiting_words, work, AfterStop, Receivers, Shown,
};
use crate::state::{stick_hex, AppState};

mod change;
use change::change_into_received;

/// How long a link is waited for.
const WAIT_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// "Wait for it": the link is kept (its ticket in the secret store), held,
/// and looked at again by itself. Returns its id.
#[tauri::command]
pub async fn wait_for_link(
    text: String,
    app: AppHandle,
    state: State<'_, AppState>,
    receivers: State<'_, Receivers>,
) -> Result<String, String> {
    let ticket = ticket_of(&text)?;
    let stick_id = ticket.stick_id;
    if receivers.is_held(&stick_id).await {
        return Err("This stick is being added already.".to_owned());
    }
    let added_at_ms = SystemClock.now_ms();
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let lock = {
        let ticket = ticket.clone();
        work(move || {
            check_link(store.as_ref(), &stick_id, StickKind::App)?;
            let lock = match take_retrying(|| download_lock(&data_dir, &stick_id)) {
                Err(LockError::DownloadInUse) => anyhow::bail!(
                    "Another OurStick program on this computer is getting this stick right now. \
                     Close it, then try again."
                ),
                lock => lock?,
            };
            // Kept only where secrets are safe; never in a plain file.
            let secrets = crate::keys::secrets(store.as_ref())?;
            ourstick_core::check_secret_store(secrets.as_ref())?;
            crate::keys::ensure_device(store.as_ref(), &data_dir)?;
            keep_ticket(secrets.as_ref(), &ticket)?;
            let link = WaitingLink {
                stick_id,
                added_at_ms,
            };
            if let Err(error) = store.add_waiting_link(&link) {
                let _ = secrets.delete(&SecretName::Ticket(stick_id));
                return Err(error.into());
            }
            Ok(lock)
        })
        .await?
    };
    crate::local_network::remember(&state);
    receivers.hold(stick_id, Some(lock)).await;
    start_waiting(&app, stick_id, ticket, added_at_ms, FOLLOW_EVERY, None).await;
    changed(&app);
    Ok(stick_hex(&stick_id))
}

/// Starts (again) the wait for the held `stick_id`, first after `first`,
/// shown with `message` until there is news: nothing if it is not held
/// (forgotten meanwhile) or the app is quitting. Boxed: a failed change
/// into a received stick starts it again.
pub fn start_waiting(
    app: &AppHandle,
    stick_id: StickId,
    ticket: Ticket,
    added_at_ms: u64,
    first: Duration,
    message: Option<String>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
    Box::pin(async move {
        let receivers = app.state::<Receivers>();
        let mut held = receivers.held.lock().await;
        if receivers.quitting.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let Some(entry) = held.get_mut(&stick_id) else {
            return;
        };
        entry.stop().await;
        show_waiting(app, stick_id, added_at_ms, message);
        let cancel = Cancel::new();
        let task = {
            let app = app.clone();
            let cancel = cancel.clone();
            tauri::async_runtime::spawn(async move {
                wait(&app, stick_id, ticket, added_at_ms, first, &cancel).await;
            })
        };
        entry.running = Some((cancel, task));
    })
}

/// What the window shows of a waiting link: since when, and why it waits
/// if the sender said more than "away".
fn show_waiting(app: &AppHandle, stick_id: StickId, added_at_ms: u64, message: Option<String>) {
    app.state::<Receivers>().show(stick_id, |shown| {
        *shown = Shown {
            state: "waiting_sender",
            message,
            since_ms: Some(added_at_ms),
            ..Shown::default()
        }
    });
    changed(app);
}

/// One wait, until the sender answers (then the change into a received
/// stick, in a task of its own), 30 days pass, or it is cancelled.
async fn wait(
    app: &AppHandle,
    stick_id: StickId,
    ticket: Ticket,
    added_at_ms: u64,
    first: Duration,
    cancel: &Cancel,
) {
    let ends_at_ms = added_at_ms.saturating_add(WAIT_FOR.as_millis() as u64);
    let left = Duration::from_millis(ends_at_ms.saturating_sub(SystemClock.now_ms()));
    let waited = tokio::select! {
        waited = waiting(app, stick_id, &ticket, added_at_ms, first, cancel) => waited,
        () = tokio::time::sleep(left) => return ended(app, stick_id, added_at_ms),
    };
    match waited {
        Ok((name, folder)) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                change_into_received(&app, stick_id, ticket, added_at_ms, name, folder).await;
            });
        }
        Err(DownloadError::Cancelled) => {}
        Err(error) => {
            let words = stopped_words_then(&error, AfterStop::AddAgain);
            show_waiting(app, stick_id, added_at_ms, Some(words))
        }
    }
}

/// The 30 days passed: no more tries; only Forget is left.
fn ended(app: &AppHandle, stick_id: StickId, added_at_ms: u64) {
    app.state::<Receivers>().show(stick_id, |shown| {
        *shown = Shown {
            state: "wait_ended",
            message: Some("The sender did not answer for 30 days.".to_owned()),
            since_ms: Some(added_at_ms),
            ..Shown::default()
        }
    });
    changed(app);
}

/// Looks for the sender after `first`, then as a receive waits, with one
/// throwaway fetcher: its name and the folder it goes in once it answers.
async fn waiting(
    app: &AppHandle,
    stick_id: StickId,
    ticket: &Ticket,
    added_at_ms: u64,
    first: Duration,
    cancel: &Cancel,
) -> Result<(String, PathBuf), DownloadError> {
    tokio::select! {
        () = tokio::time::sleep(first) => {}
        () = cancel.cancelled() => return Err(DownloadError::Cancelled),
    }
    let state = app.state::<AppState>();
    // A command-line receive of it goes on in its folder.
    let earlier = {
        let store = state.store.clone();
        work(move || {
            Ok(store
                .joined_stick(&stick_id)?
                .map(|joined| joined.output_dir))
        })
        .await
        .unwrap_or(None)
    };
    let choose = match super::preview::folder_choice(earlier, None) {
        Ok(choose) => choose,
        Err(why) => {
            show_waiting(app, stick_id, added_at_ms, Some(why));
            cancel.cancelled().await;
            return Err(DownloadError::Cancelled);
        }
    };
    let hex: String = random_key()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let blobs = state.data_dir.join("previews").join(hex);
    let config = IrohConfig::new(random_key(), Network::Public);
    crate::local_network::may_bind(&state).await;
    let fetcher = match IrohFetcher::start(&config, &blobs).await {
        Ok(fetcher) => fetcher,
        Err(error) => {
            tracing::warn!(%error, "a waiting link's networking");
            show_waiting(
                app,
                stick_id,
                added_at_ms,
                Some("This computer's network could not be started for it.".to_owned()),
            );
            cancel.cancelled().await;
            return Err(DownloadError::Cancelled);
        }
    };
    let mut on_wait = |why: &Waiting| {
        let message = match why {
            Waiting::Sender(_) => None,
            other => Some(waiting_words(other)),
        };
        show_waiting(app, stick_id, added_at_ms, message);
    };
    let waited = wait_for_owner(
        &fetcher,
        state.store.clone(),
        ticket,
        choose,
        super::preview::PREVIEW_WAIT,
        cancel,
        &mut on_wait,
    )
    .await;
    let _ = fetcher.shutdown(false).await;
    let _ = tauri::async_runtime::spawn_blocking(move || std::fs::remove_dir_all(&blobs)).await;
    waited.map(|preview| (preview.name, preview.folder))
}

/// "Forget this stick…" on a waiting link, already released (its wait
/// stopped): its row, then its ticket (one left behind is harmless).
pub(super) async fn forget_waiting(state: &AppState, stick_id: StickId) -> Result<(), String> {
    let store = state.store.clone();
    work(move || {
        store.remove_waiting_link(&stick_id)?;
        let secrets = crate::keys::secrets(store.as_ref())?;
        secrets.delete(&SecretName::Ticket(stick_id))?;
        Ok(())
    })
    .await
}

/// Whether `stick_id` is a link the app waits for.
pub(super) async fn is_waiting(state: &AppState, stick_id: StickId) -> Result<bool, String> {
    let store = state.store.clone();
    work(move || {
        Ok(store
            .waiting_links()?
            .iter()
            .any(|link| link.stick_id == stick_id))
    })
    .await
}

/// At start: every link waited for held (with its lock, if this program
/// can take it) and looked at again from the first pause.
pub(super) async fn hold_all(
    app: &AppHandle,
    waits: Vec<(
        WaitingLink,
        Result<ourstick_core::locks::Lock, LockError>,
        anyhow::Result<Ticket>,
    )>,
) {
    let receivers = app.state::<Receivers>();
    for (link, lock, ticket) in waits {
        let stick_id = link.stick_id;
        let lock = match lock {
            Ok(lock) => Some(lock),
            Err(error) => {
                super::stopped(app, stick_id, super::lock_words(&error));
                None
            }
        };
        let without_lock = lock.is_none();
        receivers.hold(stick_id, lock).await;
        if without_lock {
            continue;
        }
        match ticket {
            Ok(ticket) => {
                start_waiting(app, stick_id, ticket, link.added_at_ms, FOLLOW_EVERY, None).await
            }
            Err(error) => super::stopped(app, stick_id, super::said(&error)),
        }
    }
}
