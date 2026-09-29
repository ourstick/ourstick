//! What the app keeps while it runs.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use ourstick_core::device;
use ourstick_core::{OwnedStick, Store};
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

    /// The app's stick: the owned stick that is a drive, if there is one yet.
    pub fn drive_stick(&self) -> Result<Option<OwnedStick>> {
        Ok(self
            .store
            .owned_sticks()?
            .into_iter()
            .find(|stick| stick.drive_size.is_some()))
    }
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
