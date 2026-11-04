//! The window's commands for received sticks (step 7b): the list, a pasted
//! sharing link's preview, adding it, pause, resume, forget. A pasted link
//! is never shown back, logged or put in a message: it holds the stick's key.

use std::path::PathBuf;

use ourstick_core::clock::{Clock, SystemClock};
use ourstick_core::download::{mark_received_folder, take_stick};
use ourstick_core::locks::{download_lock, take_retrying, LockError};
use ourstick_core::places::check_received_folder;
use ourstick_core::received::{check_link, forget_received};
use ourstick_core::report::ReportState;
use ourstick_core::store::JoinedChange;
use ourstick_core::{StickKind, Ticket};
use tauri::{AppHandle, Emitter, State};

use super::{keep_ticket, read_ticket, start, Receivers};
use crate::state::{stick_hex, stick_id as stick_id_of, AppState};

/// The pasted text as a sharing link. The error never repeats it.
pub(super) fn ticket_of(text: &str) -> Result<Ticket, String> {
    text.trim().parse().map_err(|_| {
        "This link is not whole. Copy all of it from the message: it is long.".to_owned()
    })
}

/// "Choose…": the system's folder picker; the folder the stick goes in.
#[cfg(desktop)]
#[tauri::command]
pub async fn choose_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a folder")
            .blocking_pick_folder()
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(picked
        .and_then(|picked| picked.into_path().ok())
        .map(|path| path.display().to_string()))
}

/// A phone keeps what it receives in one place (ADR 0027): no picker.
#[cfg(mobile)]
#[tauri::command]
pub async fn choose_folder(_app: AppHandle) -> Result<Option<String>, String> {
    Err("A phone keeps what it receives in one folder.".to_owned())
}

/// "Get it": the stick is received into `folder` from now on, and kept up
/// to date. Returns its id.
#[tauri::command]
pub async fn add_received(
    text: String,
    folder: String,
    name: String,
    left_out: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
    receivers: State<'_, Receivers>,
) -> Result<String, String> {
    let ticket = ticket_of(&text)?;
    let stick_id = ticket.stick_id;
    if receivers.is_held(&stick_id).await {
        return Err("This stick is being added already.".to_owned());
    }
    let folder = PathBuf::from(folder);
    if let Some(why) = super::into::refused(&folder, super::into::home().as_deref()) {
        return Err(why.to_owned());
    }
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let made = {
        let ticket = ticket.clone();
        let folder = folder.clone();
        super::work(move || {
            check_link(store.as_ref(), &stick_id, StickKind::App)?;
            // Its lock first: nothing else receives it while it is added.
            let lock = match take_retrying(|| download_lock(&data_dir, &stick_id)) {
                Err(LockError::DownloadInUse) => anyhow::bail!(
                    "Another OurStick program on this computer is getting this stick right now. \
                     Close it, then try again."
                ),
                lock => lock?,
            };
            check_received_folder(store.as_ref(), &data_dir, &folder, &stick_id)?;
            std::fs::create_dir_all(&folder)?;
            mark_received_folder(&folder, &stick_id)?;
            let keys = || {
                // Kept only where secrets are safe; never in a plain file.
                let secrets = crate::keys::secrets(store.as_ref())?;
                ourstick_core::check_secret_store(secrets.as_ref())?;
                // Recorded before the first key, so all keys go to one store.
                crate::keys::ensure_device(store.as_ref(), &data_dir)?;
                keep_ticket(secrets.as_ref(), &ticket)
            };
            if let Err(error) = keys() {
                // Nothing of it left: the next try makes the same folder
                // again, not "<name> (2)" beside an empty one.
                ourstick_core::download::unmark_received_folder(&folder, &stick_id);
                let _ = std::fs::remove_dir(&folder);
                return Err(error);
            }
            Ok(lock)
        })
        .await?
    };
    // The device may be new: the Local Network step is kept with it.
    crate::local_network::remember(&state);
    // Held at once: from here on, nothing else receives it.
    receivers.hold(stick_id, Some(made)).await;
    let now = SystemClock.now_ms();
    if let Err(error) = take_stick(&state.store, stick_id, &folder, StickKind::App, now).await {
        receivers.release(&stick_id).await;
        tracing::warn!(%error, "a received stick could not be recorded");
        let words = match error {
            ourstick_core::download::DownloadError::BeingForgotten => {
                "This stick is still being forgotten. Restart OurStick, then add it again."
            }
            _ => "It could not be added. Try again in a moment.",
        };
        // Nothing of it left: its mark, its folder if still empty, and its
        // saved sharing link go.
        let made_folder = folder.clone();
        let store = state.store.clone();
        let _ = super::work(move || {
            ourstick_core::download::unmark_received_folder(&made_folder, &stick_id);
            let _ = std::fs::remove_dir(&made_folder);
            let secrets = crate::keys::secrets(store.as_ref())?;
            secrets.delete(&ourstick_core::secrets::SecretName::Ticket(stick_id))?;
            Ok(())
        })
        .await;
        return Err(words.to_owned());
    }
    // What the user un-ticked in the preview, before its first round
    // (ADR 0012). If it cannot be kept, the stick is not added at all (as if
    // it could not be recorded): nothing is got that was not wanted, then or
    // at a later start.
    if !left_out.is_empty() {
        let store = state.store.clone();
        let names = left_out.clone();
        let chosen = super::work(move || Ok(store.set_left_out(&stick_id, &names)?)).await;
        if chosen.is_err() {
            let lock = receivers.release(&stick_id).await;
            let store = state.store.clone();
            let data_dir = state.data_dir.clone();
            let made_folder = folder.clone();
            let undone = super::work(move || {
                // First what needs no store (it may be what failed): without
                // its mark the folder is never written to, now or at a later
                // start.
                ourstick_core::download::unmark_received_folder(&made_folder, &stick_id);
                let _ = std::fs::remove_dir(&made_folder);
                let secrets = crate::keys::secrets(store.as_ref())?;
                if let Some(lock) = &lock {
                    forget_received(store.as_ref(), secrets.as_ref(), &data_dir, &stick_id, lock)?;
                }
                Ok(())
            })
            .await;
            let mut words = "What not to get could not be saved, so the stick was not added. \
                             Try again in a moment.";
            if let Err(why) = undone {
                tracing::warn!(%why, "a stick whose choice could not be kept, half undone");
                // Still listed: shown, saying so, for the user to forget it.
                // Already marked forgotten: hidden, finished at the next start.
                let store = state.store.clone();
                let listed = super::work(move || Ok(store.joined_stick(&stick_id)?.is_some()))
                    .await
                    .unwrap_or(true);
                if listed {
                    words = "What not to get could not be saved. Nothing is being got: use \
                             Forget this stick…, then add it again.";
                    receivers
                        .hold_again(&app, stick_id, None, words.to_owned())
                        .await;
                }
            }
            super::changed(&app);
            return Err(words.to_owned());
        }
    }
    // Its name as the preview showed it, if it is a name a manifest may
    // carry; the first round keeps it as the sender gives it anyway.
    if ourstick_core::manifest::validate_name(&name).is_ok() {
        let store = state.store.clone();
        let named = super::work(move || {
            Ok(store.change_joined_stick(&stick_id, &JoinedChange::Name(name))?)
        })
        .await;
        if let Err(error) = named {
            tracing::warn!(%error, "a received stick's name waits for its first round");
        }
    }
    start(&app, stick_id, ticket, folder, false).await;
    Ok(stick_hex(&stick_id))
}

/// "Pause": stops receiving it (remembered after a restart); its files and
/// what came so far stay.
#[tauri::command]
pub async fn pause_received(
    stick_id: String,
    app: AppHandle,
    receivers: State<'_, Receivers>,
) -> Result<(), String> {
    let stick_id = stick_id_of(&stick_id)?;
    let paused = receivers.pause(&app, &stick_id).await;
    let _ = app.emit("received-changed", ());
    if paused.is_ok() {
        // Told in the background: the window does not wait for the owner.
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            super::reports::report_once(&app, stick_id, ReportState::Paused).await;
        });
    }
    paused
}

/// "Resume": received again, from where it was.
#[tauri::command]
pub async fn resume_received(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
    receivers: State<'_, Receivers>,
) -> Result<(), String> {
    let stick_id = stick_id_of(&stick_id)?;
    if !receivers.is_held(&stick_id).await {
        return Err("this stick is not received here any more".to_owned());
    }
    let store = state.store.clone();
    let (ticket, folder) = super::work(move || {
        let joined = store
            .joined_stick(&stick_id)?
            .ok_or_else(|| anyhow::anyhow!("this stick is not received here any more"))?;
        let secrets = crate::keys::secrets(store.as_ref())?;
        Ok((read_ticket(secrets.as_ref(), &stick_id)?, joined.output_dir))
    })
    .await?;
    start(&app, stick_id, ticket, folder, true).await;
    Ok(())
}

/// "Forget this stick": no longer received or kept up to date; its files
/// stay in its folder.
#[tauri::command]
pub async fn forget_received_stick(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
    receivers: State<'_, Receivers>,
) -> Result<(), String> {
    let stick_id = stick_id_of(&stick_id)?;
    // Released first: a wait that was changing it into a received stick
    // has finished by then, or never will (ADR 0025).
    let held = receivers.release(&stick_id).await;
    if super::waiting::is_waiting(&state, stick_id).await? {
        let forgotten = super::waiting::forget_waiting(&state, stick_id).await;
        if forgotten.is_err() {
            receivers
                .hold_again(
                    &app,
                    stick_id,
                    held,
                    "Forgetting it did not finish. Use Forget this stick… again.".to_owned(),
                )
                .await;
        }
        let _ = app.emit("received-changed", ());
        return forgotten;
    }
    // Before its key and link go: at most a few seconds, then forgotten
    // whether the owner heard or not.
    super::reports::report_once(&app, stick_id, ReportState::Forgotten).await;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let forgotten = super::work(move || {
        let lock = match held {
            Some(lock) => lock,
            None => take_retrying(|| download_lock(&data_dir, &stick_id))?,
        };
        let secrets = crate::keys::secrets(store.as_ref())?;
        Ok(forget_received(
            store.as_ref(),
            secrets.as_ref(),
            &data_dir,
            &stick_id,
            &lock,
        )?)
    })
    .await;
    if let Err(why) = &forgotten {
        // Still listed (the forget stopped before it was marked): held
        // again, saying why. Already marked: hidden, and finished at the
        // next start; said for the whole app.
        let store = state.store.clone();
        let listed = super::work(move || Ok(store.joined_stick(&stick_id)?.is_some()))
            .await
            .unwrap_or(false);
        if listed {
            receivers
                .hold_again(
                    &app,
                    stick_id,
                    None,
                    format!("Forgetting it did not finish. {why}"),
                )
                .await;
        } else {
            crate::problems::failed(
                &app,
                None,
                format!("Forgetting a stick did not finish; OurStick finishes it at its next start. {why}"),
            );
        }
    }
    let _ = app.emit("received-changed", ());
    forgotten
}

/// "Open folder".
#[tauri::command]
pub async fn open_received_folder(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let stick_id = stick_id_of(&stick_id)?;
    let store = state.store.clone();
    let joined = super::work(move || {
        store
            .joined_stick(&stick_id)?
            .ok_or_else(|| anyhow::anyhow!("this stick is not received here any more"))
    })
    .await?;
    crate::commands::open_folder(&app, joined.output_dir).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_link_is_refused_without_repeating_it() {
        let secret = "https://ourstick.com/s/#abc.very-secret-key";
        let refused = ticket_of(secret).unwrap_err();
        assert!(!refused.contains("very-secret-key"), "{refused}");
        assert!(refused.contains("link is not whole"));
    }
}
