//! The commands the UI may call. Each one calls only `ourstick-core`, and its
//! errors reach the UI as text meant for users.

use anyhow::{bail, Result};
use ourstick_core::device;
use ourstick_core::secrets::check_secret_store;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::drive;
use crate::sharing::{mount_and_share, start_sharing_later};
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
    first_launch: bool,
    /// The Local Network step comes first (ADR 0020, macOS).
    explain_local_network: bool,
}

#[tauri::command]
pub async fn app_state(state: State<'_, AppState>) -> Result<AppStateView, String> {
    // Neither a stick of one's own nor one received in the window.
    let store = state.store.clone();
    let receives = blocking(move || {
        Ok(store
            .joined_sticks()?
            .iter()
            .any(|stick| stick.kind == ourstick_core::StickKind::App))
    })
    .await?;
    let first_launch = state.sticks_now().await.map_err(shown)?.is_empty() && !receives;
    let explain_local_network = {
        let state = state.inner();
        crate::local_network::must_explain(state)
    };
    Ok(AppStateView {
        platform: std::env::consts::OS,
        first_launch,
        explain_local_network,
    })
}

/// First launch: this device's keys (made once), and the first stick,
/// called `name`. It has no size: what it holds is what the computer has
/// room for (ADR 0008).
#[tauri::command]
pub async fn plug_in(
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if !state.sticks_now().await.map_err(shown)?.is_empty() {
        return Ok(());
    }
    let data_dir = state.data_dir.clone();
    let store = state.store.clone();
    let explained = state
        .local_network_explained
        .load(std::sync::atomic::Ordering::Relaxed);
    let made = blocking(move || {
        let secrets = device::open_system_keyring()?;
        check_secret_store(secrets.as_ref())?;
        let Some(backend) = device::system_keyring_backend() else {
            bail!("this system has no keyring OurStick can use yet");
        };
        // One program makes the device and the stick at a time.
        let _setup = ourstick_core::locks::setup_lock(&data_dir)?;
        device::open_device(store.as_ref(), &data_dir, secrets.as_ref(), backend)?;
        if explained {
            store.set_local_network_explained()?;
        }
        if !crate::state::app_sticks(store.as_ref())?.is_empty() {
            return Ok(None);
        }
        let name = ourstick_core::stick_names::new_stick_name(&name, &[])?;
        let made = crate::making::make_stick(store.as_ref(), secrets.as_ref(), &data_dir, &name)?;
        Ok(Some(made))
    })
    .await?;
    let _ = app.emit("mount-changed", ());
    if let Some(made) = made {
        crate::stick_life_commands::tell_unplugged(&app, made.stick_id, made.unplugged);
        start_sharing_later(&app, made.stick_id);
    }
    Ok(())
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
    use tauri_plugin_opener::OpenerExt;
    let stick = state
        .stick_now(&stick_id_of(&stick_id)?)
        .await
        .map_err(shown)?;
    let shown = blocking(move || Ok(drive::shown_at(&stick.root_path))).await?;
    app.opener()
        .open_path(shown.to_string_lossy(), None::<&str>)
        .map_err(|error| error.to_string())
}
