//! The commands the UI may call. Each one calls only `ourstick-core`, and its
//! errors reach the UI as text meant for users.

use anyhow::{bail, Context, Result};
use ourstick_core::device;
use ourstick_core::secrets::check_secret_store;
use ourstick_core::share::mark_drive;
use serde::Serialize;
use tauri::State;

use crate::drive;
use crate::state::AppState;

/// The stick sizes offered at first launch, in GB (docs/gui-spec.md).
const STICK_SIZES_GB: [u64; 4] = [5, 10, 20, 50];

/// An error as the UI shows it.
fn shown(error: anyhow::Error) -> String {
    format!("{error:#}")
}

/// Runs blocking core work off the async runtime (CLAUDE.md rule 7).
async fn blocking<T: Send + 'static>(
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

/// First launch: this device's keys (made once), and the stick's folder with
/// the chosen size.
#[tauri::command]
pub async fn plug_in(size_gb: u64, state: State<'_, AppState>) -> Result<(), String> {
    if !STICK_SIZES_GB.contains(&size_gb) {
        return Err(format!("{size_gb} GB is not one of the stick sizes"));
    }
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
        device::open_device(store.as_ref(), &data_dir, secrets.as_ref(), backend)?;
        let root = drive::create()?;
        let keys = device::create_stick(
            store.as_ref(),
            secrets.as_ref(),
            &root,
            "OurStick",
            Some(size_gb * 1_000_000_000),
        )?;
        mark_drive(&root, &keys.stick.stick_id).context("cannot mark the stick's folder")?;
        Ok(())
    })
    .await
}
