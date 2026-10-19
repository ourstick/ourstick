//! The commands the UI may call. Each one calls only `ourstick-core`, and its
//! errors reach the UI as text meant for users.

use anyhow::Result;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::drive;
use crate::sharing::mount_and_share;
use crate::state::{stick_id as stick_id_of, AppState};

/// An error as the UI shows it.
pub(crate) fn shown(error: anyhow::Error) -> String {
    crate::words::told(&error)
}

/// Runs blocking core work off the async runtime (CLAUDE.md rule 7).
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
        .map_err(shown)
}

#[derive(Debug, Serialize)]
pub struct AppStateView {
    platform: &'static str,
    /// The Local Network step comes first (ADR 0020, macOS).
    explain_local_network: bool,
    /// The keys' move to the data protection keychain comes first, with how
    /// many keys macOS asks about (ADR 0021).
    move_keys: Option<usize>,
}

#[tauri::command]
pub async fn app_state(state: State<'_, AppState>) -> Result<AppStateView, String> {
    let explain_local_network = !*state.network_open.borrow();
    let move_keys = if state.keys_wait.load(std::sync::atomic::Ordering::Relaxed) {
        let store = state.store.clone();
        blocking(move || crate::keys::move_due(store.as_ref())).await?
    } else {
        None
    };
    Ok(AppStateView {
        platform: std::env::consts::OS,
        explain_local_network,
        move_keys,
    })
}

/// Ejects a stick (detaches the image, or takes it out of the sidebar).
/// Sharing goes on with the last version.
#[tauri::command]
pub async fn eject(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id = stick_id_of(&stick_id)?;
    if crate::sharing::is_busy(&state, &id).await {
        return Err("the stick is busy; try again in a moment".to_owned());
    }
    // Ejected under a copy, the copy would fail half way.
    if crate::stick_drop::is_copying(&state, &id) {
        return Err("a copy onto the stick is still going on; eject it once it is done".to_owned());
    }
    let stick = state.stick_now(&id).await.map_err(shown)?;
    blocking(move || drive::eject(&stick)).await?;
    let _ = app.emit("mount-changed", ());
    crate::sharing::refresh_tray(&app).await;
    Ok(())
}

/// Mounts a stick again, and shares it from where it is now.
#[tauri::command]
pub fn mount(stick_id: String, app: AppHandle) -> Result<(), String> {
    mount_and_share(&app, stick_id_of(&stick_id)?);
    Ok(())
}

#[tauri::command]
pub async fn open_in_file_manager(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let stick = state
        .stick_now(&stick_id_of(&stick_id)?)
        .await
        .map_err(shown)?;
    let shown = blocking(move || Ok(drive::shown_at(&stick.root_path))).await?;
    open_folder(&app, shown).await
}

/// The keys' step (ADR 0021): `later`, the start goes on with the login
/// keychain for this run; otherwise the keys move first. A failure keeps
/// them where they were and says why; the step stays for "Try again".
#[tauri::command]
pub async fn move_keys(
    later: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if !later {
        let store = state.store.clone();
        let data_dir = state.data_dir.clone();
        blocking(move || crate::keys::move_now(store.as_ref(), &data_dir)).await?;
    }
    if state.keys_wait.swap(false, Ordering::Relaxed) {
        crate::sharing::start_app(&app);
    }
    Ok(())
}

/// Opens `folder` in the file manager, saying in plain words why not: gone
/// (deleted, or its disk unplugged), or the file manager refused (its own
/// words go to the log, not to the user).
pub(crate) async fn open_folder(app: &AppHandle, folder: std::path::PathBuf) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let checked = folder.clone();
    if !blocking(move || Ok(checked.is_dir())).await? {
        return Err(not_there());
    }
    app.opener()
        .open_path(folder.to_string_lossy(), None::<&str>)
        .map_err(|error| {
            tracing::warn!(%error, "the file manager did not open a folder");
            "The folder could not be opened in the file manager.".to_owned()
        })
}

fn not_there() -> String {
    "This folder is not there any more (deleted or moved, or its disk is unplugged).".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_that_is_not_there_is_said_in_plain_words() {
        assert!(not_there().starts_with("This folder is not there"));
    }
}
