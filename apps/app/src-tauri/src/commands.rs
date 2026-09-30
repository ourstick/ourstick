//! The commands the UI may call. Each one calls only `ourstick-core`, and its
//! errors reach the UI as text meant for users.

use anyhow::{bail, Context, Result};
use ourstick_core::device;
use ourstick_core::secrets::check_secret_store;
use ourstick_core::share::mark_drive;
use std::path::PathBuf;

use ourstick_core::drive::{copy_onto, remove_from, top_entries};
use ourstick_core::share::folder_size;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::drive;
use crate::state::{start_sharing, AppState, Sharing};

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
pub async fn plug_in(
    size_gb: u64,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
        let root = drive::create(size_gb * 1_000_000_000)?;
        let keys = device::create_stick(
            store.as_ref(),
            secrets.as_ref(),
            &root,
            "OurStick",
            Some(size_gb * 1_000_000_000),
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

#[derive(Debug, Serialize)]
pub struct EntryView {
    name: String,
    size: u64,
    is_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct SharingView {
    /// "off", "starting", "on" or "failed".
    state: &'static str,
    /// Why it failed.
    message: Option<String>,
    /// The link can be shown (never sent anywhere by the app itself).
    has_link: bool,
    readers: usize,
    bytes_per_second: u64,
    /// What to do when readers come only through the relay.
    relay_only: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct StickView {
    name: String,
    /// The stick's size in bytes.
    size: u64,
    /// Bytes on it now.
    used: u64,
    entries: Vec<EntryView>,
    sharing: SharingView,
    /// Mounted (macOS) or in the sidebar (Linux); not after Eject.
    mounted: bool,
}

/// The stick as the main window shows it.
#[tauri::command]
pub async fn stick(state: State<'_, AppState>) -> Result<StickView, String> {
    let stick = state
        .drive_stick()
        .map_err(shown)?
        .ok_or("there is no stick yet")?;
    let size = stick.drive_size.unwrap_or_default();
    let root = stick.root_path.clone();
    let (entries, used, mounted) = blocking(move || {
        let mounted = drive::is_mounted(&root);
        // An ejected image has no folder: show it empty, not as an error.
        if !drive::folder_is_there(&root) {
            return Ok((Vec::new(), 0, mounted));
        }
        Ok((top_entries(&root)?, folder_size(&root)?, mounted))
    })
    .await?;
    let sharing = match &*state.sharing.lock().await {
        Sharing::Off => sharing_view("off", None, false, None),
        Sharing::Starting => sharing_view("starting", None, false, None),
        Sharing::Failed(message) => sharing_view("failed", Some(message.clone()), false, None),
        Sharing::On(serving) => sharing_view(
            "on",
            None,
            serving.link.is_some(),
            Some(*serving.activity.borrow()),
        ),
    };
    Ok(StickView {
        name: stick.name,
        size,
        used,
        entries: entries
            .into_iter()
            .map(|entry| EntryView {
                name: entry.name,
                size: entry.size,
                is_dir: entry.is_dir,
            })
            .collect(),
        sharing,
        mounted,
    })
}

fn sharing_view(
    state: &'static str,
    message: Option<String>,
    has_link: bool,
    activity: Option<ourstick_core::transport::ReaderActivity>,
) -> SharingView {
    let activity = crate::serving::ActivityView::from(activity.unwrap_or_default());
    SharingView {
        state,
        message,
        has_link,
        readers: activity.readers,
        bytes_per_second: activity.bytes_per_second,
        relay_only: activity.relay_only,
    }
}

/// Dropped files and folders: copied onto the stick, like onto a real one.
#[tauri::command]
pub async fn add_paths(
    paths: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let stick = state
        .drive_stick()
        .map_err(shown)?
        .ok_or("there is no stick yet")?;
    let size = stick.drive_size.unwrap_or_default();
    let sources: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    blocking(move || {
        if !drive::folder_is_there(&stick.root_path) {
            bail!("the stick is ejected; mount it first");
        }
        Ok(copy_onto(&stick.root_path, size, &sources)?)
    })
    .await?;
    // The watcher publishes; the window can show the files at once.
    let _ = app.emit("stick-changed", ());
    Ok(())
}

/// Removes a file or folder from the stick (its copy on the stick).
#[tauri::command]
pub async fn remove_entry(
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let stick = state
        .drive_stick()
        .map_err(shown)?
        .ok_or("there is no stick yet")?;
    blocking(move || Ok(remove_from(&stick.root_path, &name)?)).await?;
    let _ = app.emit("stick-changed", ());
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
