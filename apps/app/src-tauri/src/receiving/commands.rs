//! The window's commands for received sticks (step 7b): the list, a pasted
//! sharing link's preview, adding it, pause, resume, forget. A pasted link
//! is never shown back, logged or put in a message: it holds the stick's key.

use std::path::PathBuf;

use ourstick_core::clock::{Clock, SystemClock};
use ourstick_core::device::open_system_keyring;
use ourstick_core::download::{mark_received_folder, take_stick};
use ourstick_core::locks::{download_lock, take_retrying, LockError};
use ourstick_core::places::check_received_folder;
use ourstick_core::received::{check_link, display_name, forget_received};
use ourstick_core::store::JoinedChange;
use ourstick_core::{StickKind, Ticket};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::{keep_ticket, read_ticket, start, Receivers, Shown};
use crate::commands::{blocking, shown};
use crate::state::{stick_hex, stick_id as stick_id_of, AppState};

/// The pasted text as a sharing link. The error never repeats it.
pub(super) fn ticket_of(text: &str) -> Result<Ticket, String> {
    text.trim().parse().map_err(|_| {
        "That is not an OurStick sharing link. Copy the whole link the sender sent.".to_owned()
    })
}

/// One received stick, as the list shows it.
#[derive(Debug, Serialize)]
pub struct ReceivedEntry {
    id: String,
    name: String,
    folder: String,
    paused: bool,
    #[serde(flatten)]
    shown: Shown,
}

/// The sticks the window receives, oldest first.
#[tauri::command]
pub async fn received(
    state: State<'_, AppState>,
    receivers: State<'_, Receivers>,
) -> Result<Vec<ReceivedEntry>, String> {
    let store = state.store.clone();
    let joined = blocking(move || Ok(store.joined_sticks()?)).await?;
    Ok(joined
        .into_iter()
        .filter(|joined| joined.kind == StickKind::App)
        .map(|joined| ReceivedEntry {
            id: stick_hex(&joined.stick_id),
            name: display_name(joined.name.as_deref(), &joined.stick_id),
            folder: joined.output_dir.display().to_string(),
            paused: joined.paused,
            shown: receivers.shown(&joined.stick_id),
        })
        .collect())
}

/// "Choose…": the system's folder picker; the folder the stick goes in.
#[tauri::command]
pub async fn choose_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Where to save the stick")
            .blocking_pick_folder()
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(picked
        .and_then(|picked| picked.into_path().ok())
        .map(|path| path.display().to_string()))
}

/// "Get it": the stick is received into `folder` from now on, and kept up
/// to date. Returns its id.
#[tauri::command]
pub async fn add_received(
    text: String,
    folder: String,
    name: String,
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
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let made = {
        let ticket = ticket.clone();
        let folder = folder.clone();
        blocking(move || {
            check_link(store.as_ref(), &stick_id, StickKind::App)?;
            // Its lock first: nothing else receives it while it is added.
            let lock = match take_retrying(|| download_lock(&data_dir, &stick_id)) {
                Err(LockError::DownloadInUse) => anyhow::bail!(
                    "OurStick's command line (`ourstick get`) is receiving this stick right \
                     now; stop it first"
                ),
                lock => lock?,
            };
            check_received_folder(store.as_ref(), &data_dir, &folder, &stick_id)?;
            std::fs::create_dir_all(&folder)?;
            mark_received_folder(&folder, &stick_id)?;
            // Kept only where secrets are safe; never in a plain file.
            let secrets = open_system_keyring()?;
            ourstick_core::check_secret_store(secrets.as_ref())?;
            keep_ticket(secrets.as_ref(), &ticket)?;
            Ok(lock)
        })
        .await?
    };
    // Held at once: from here on, nothing else receives it.
    receivers.hold(stick_id, Some(made)).await;
    let now = SystemClock.now_ms();
    if let Err(error) = take_stick(&state.store, stick_id, &folder, StickKind::App, now).await {
        receivers.release(&stick_id).await;
        // Nothing of it left: its mark and its saved sharing link go.
        let marked = folder.join(ourstick_core::download::RECEIVED_MARKER);
        let _ = blocking(move || {
            let _ = std::fs::remove_file(marked);
            let secrets = open_system_keyring()?;
            secrets.delete(&ourstick_core::secrets::SecretName::Ticket(stick_id))?;
            Ok(())
        })
        .await;
        return Err(crate::receiving::stopped_words(&error));
    }
    // Its name as the preview showed it, if it is a name a manifest may
    // carry; the first round keeps it as the sender gives it anyway.
    if ourstick_core::manifest::validate_name(&name).is_ok() {
        let store = state.store.clone();
        let named =
            blocking(move || Ok(store.change_joined_stick(&stick_id, &JoinedChange::Name(name))?))
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
    let (ticket, folder) = blocking(move || {
        let joined = store
            .joined_stick(&stick_id)?
            .ok_or_else(|| anyhow::anyhow!("this stick is not received here any more"))?;
        let secrets = open_system_keyring()?;
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
    let held = receivers.release(&stick_id).await;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let forgotten = blocking(move || {
        let lock = match held {
            Some(lock) => lock,
            None => take_retrying(|| download_lock(&data_dir, &stick_id))?,
        };
        let secrets = open_system_keyring()?;
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
        // Still received here (its row stays): held again, saying why.
        receivers
            .hold_again(
                &app,
                stick_id,
                None,
                format!("Forgetting it did not finish: {why}"),
            )
            .await;
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
    use tauri_plugin_opener::OpenerExt;
    let stick_id = stick_id_of(&stick_id)?;
    let store = state.store.clone();
    let joined = blocking(move || {
        store
            .joined_stick(&stick_id)?
            .ok_or_else(|| anyhow::anyhow!("this stick is not received here any more"))
    })
    .await?;
    app.opener()
        .open_path(joined.output_dir.to_string_lossy(), None::<&str>)
        .map_err(|error| shown(error.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_link_is_refused_without_repeating_it() {
        let secret = "https://ourstick.com/s/abc#very-secret-key";
        let refused = ticket_of(secret).unwrap_err();
        assert!(!refused.contains("very-secret-key"), "{refused}");
        assert!(refused.contains("sharing link"));
    }
}
