//! The commands the UI may call. Each one calls only `ourstick-core`, and its
//! errors reach the UI as text meant for users.

use anyhow::{bail, Context, Result};
use ourstick_core::device;
use ourstick_core::secrets::check_secret_store;
use ourstick_core::share::mark_drive;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::drive;
use crate::state::{start_sharing, AppState};

/// An error as the UI shows it.
pub(crate) fn shown(error: anyhow::Error) -> String {
    format!("{error:#}")
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
    first_launch: bool,
}

#[tauri::command]
pub async fn app_state(state: State<'_, AppState>) -> Result<AppStateView, String> {
    let first_launch = state.drive_stick().map_err(shown)?.is_none();
    Ok(AppStateView {
        platform: std::env::consts::OS,
        first_launch,
    })
}

/// First launch: this device's keys (made once), and the stick's folder. It
/// has no size: what it holds is what the computer has room for (ADR 0008).
#[tauri::command]
pub async fn plug_in(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if state.drive_stick().map_err(shown)?.is_some() {
        return Ok(());
    }
    let data_dir = state.data_dir.clone();
    let store = state.store.clone();
    blocking(move || {
        let secrets = device::open_system_keyring()?;
        check_secret_store(secrets.as_ref())?;
        let Some(backend) = device::system_keyring_backend() else {
            bail!("this system has no keyring OurStick can use yet");
        };
        // One program makes the device and the stick at a time.
        let _setup = ourstick_core::locks::setup_lock(&data_dir)?;
        device::open_device(store.as_ref(), &data_dir, secrets.as_ref(), backend)?;
        if crate::state::app_stick(store.as_ref())?.is_some() {
            return Ok(());
        }
        let root = drive::create()?;
        let keys = device::create_stick(
            store.as_ref(),
            secrets.as_ref(),
            &root,
            "OurStick",
            ourstick_core::StickKind::App,
        )?;
        mark_drive(&root, &keys.stick.stick_id).context("cannot mark the stick's folder")?;
        // Windows: the letter it got, to ask for it again after a restart.
        if let Some(letter) = drive::letter_of(&root) {
            store.set_drive_letter(&keys.stick.stick_id, Some(letter))?;
        }
        Ok(())
    })
    .await?;
    let _ = app.emit("mount-changed", ());
    start_sharing(&app);
    Ok(())
}

/// Ejects the stick (detaches the image, or takes it out of the sidebar).
/// Sharing goes on with the last version.
#[tauri::command]
pub async fn eject(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let stick = state
        .drive_stick()
        .map_err(shown)?
        .ok_or("there is no stick yet")?;
    blocking(move || drive::eject(&stick.root_path)).await?;
    crate::tray::set_mounted(&app, false);
    let _ = app.emit("mount-changed", ());
    Ok(())
}

/// Mounts the stick again, and shares from where it is now.
#[tauri::command]
pub fn mount(app: AppHandle) {
    crate::state::plug_in_and_share(&app);
}

#[tauri::command]
pub async fn open_in_file_manager(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let stick = state
        .drive_stick()
        .map_err(shown)?
        .ok_or("there is no stick yet")?;
    let shown = blocking(move || Ok(drive::shown_at(&stick.root_path))).await?;
    app.opener()
        .open_path(shown.to_string_lossy(), None::<&str>)
        .map_err(|error| error.to_string())
}
