//! What the app keeps while it runs.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use ourstick_core::device;
use ourstick_core::{OwnedStick, StickKind, Store};
use tauri::{AppHandle, Emitter, Manager};

use crate::serving::{self, Serving};

/// Where sharing stands.
pub enum Sharing {
    /// No stick yet, or not started.
    Off,
    Starting,
    On(Serving),
    /// Could not start; the message says why.
    Failed(String),
}

/// The data folder and database (shared with the CLI on this device), and
/// the running share.
pub struct AppState {
    pub data_dir: PathBuf,
    pub store: Arc<dyn Store>,
    pub sharing: tokio::sync::Mutex<Sharing>,
}

impl AppState {
    pub fn open() -> Result<Self> {
        let data_dir = device::data_dir()?;
        let store = device::open_store(&data_dir)?;
        Ok(Self {
            data_dir,
            store,
            sharing: tokio::sync::Mutex::new(Sharing::Off),
        })
    }

    /// The app's stick, if there is one yet.
    pub fn drive_stick(&self) -> Result<Option<OwnedStick>> {
        app_stick(self.store.as_ref())
    }
}

/// The app's stick (`StickKind::App`), if there is one yet: the one place
/// that says which stick is the app's.
pub fn app_stick(store: &dyn Store) -> Result<Option<OwnedStick>> {
    Ok(store
        .owned_sticks()?
        .into_iter()
        .find(|stick| stick.kind == StickKind::App))
}

/// Plugs the stick in (mounts its image, or puts it in the sidebar), records
/// where its folder is now, and starts sharing. At start and on Mount.
pub fn plug_in_and_share(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let store = state.store.clone();
        // Whether sharing must start again to follow the folder: always
        // after a disk image was attached (its old watch is gone), on Linux
        // only if the folder moved.
        let mounted = tauri::async_runtime::spawn_blocking(move || -> Result<Option<bool>> {
            let Some(stick) = app_stick(store.as_ref())? else {
                return Ok(None);
            };
            let root = crate::drive::mount(&stick.root_path, stick.drive_letter)?;
            let moved = root != stick.root_path;
            if moved {
                store.move_owned_stick(&stick.stick_id, &root)?;
            }
            // Windows: keep the letter it got, to ask for it after a restart.
            let letter = crate::drive::letter_of(&root);
            if letter.is_some() && letter != stick.drive_letter {
                store.set_drive_letter(&stick.stick_id, letter)?;
            }
            Ok(Some(moved || cfg!(target_os = "macos")))
        })
        .await;
        let mut restart = true;
        match mounted {
            Ok(Ok(Some(follow))) => restart = follow,
            Ok(Ok(None)) => return,
            Ok(Err(error)) => {
                // Sharing still serves the last version from the cache.
                tracing::warn!(error = %format!("{error:#}"), "the stick could not be mounted");
                let _ = app.emit(
                    "share-problem",
                    serving::ShareProblem::Failed {
                        message: format!("The stick could not be mounted: {error:#}"),
                    },
                );
            }
            Err(error) => tracing::warn!(%error, "mounting did not finish"),
        }
        let _ = app.emit("mount-changed", ());
        {
            let mut sharing = state.sharing.lock().await;
            if !restart && matches!(*sharing, Sharing::On(_) | Sharing::Starting) {
                return;
            }
            // The watcher follows the folder where it is now.
            if let Sharing::On(serving) = std::mem::replace(&mut *sharing, Sharing::Off) {
                serving.stop().await;
            }
        }
        start_sharing(&app);
    });
}

/// Tells the UI (event `mount-changed`) when the stick is ejected or mounted
/// behind the app's back, e.g. with Finder's eject button. Checks every few
/// seconds while the app runs.
pub fn watch_mount(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let state = app.state::<AppState>();
            let Ok(Some(stick)) = state.drive_stick() else {
                continue;
            };
            let root = stick.root_path.clone();
            let mounted =
                tauri::async_runtime::spawn_blocking(move || crate::drive::is_mounted(&root))
                    .await
                    .unwrap_or(false);
            if last != Some(mounted) {
                if last.is_some() {
                    let _ = app.emit("mount-changed", ());
                }
                crate::tray::set_mounted(&app, mounted);
                last = Some(mounted);
            }
        }
    });
}

/// Starts sharing in the background; the UI hears `sharing-changed` when it
/// starts and when it is running or has failed.
pub fn start_sharing(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        {
            let mut sharing = state.sharing.lock().await;
            if matches!(*sharing, Sharing::Starting | Sharing::On(_)) {
                return;
            }
            *sharing = Sharing::Starting;
        }
        let _ = app.emit("sharing-changed", ());
        let started =
            serving::start(app.clone(), state.data_dir.clone(), state.store.clone()).await;
        crate::tray::set_sharing(&app, started.is_ok());
        *state.sharing.lock().await = match started {
            Ok(serving) => Sharing::On(serving),
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "sharing did not start");
                Sharing::Failed(format!("{error:#}"))
            }
        };
        let _ = app.emit("sharing-changed", ());
    });
}
