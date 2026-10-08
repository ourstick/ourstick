//! What the app keeps while it runs: the data folder, the database, and
//! where sharing stands for each of the app's sticks (ADR 0009).

use ourstick_core::disk::turns::{DiskTurns, SystemTimer};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use ourstick_core::device;
use ourstick_core::{OwnedStick, StickId, StickKind, Store};

use crate::serving::Serving;

/// Where sharing one stick stands.
pub enum Sharing {
    /// Not shared: the user stopped it, or it was never started.
    Off,
    Starting,
    On(Serving),
    /// Could not start; the message says why.
    Failed(String),
    /// Being renamed, given a new link or removed: nothing starts it
    /// meanwhile. Says what is happening.
    Busy(&'static str),
}

impl Sharing {
    /// Whether it serves, or is about to.
    pub fn is_on(&self) -> bool {
        matches!(self, Self::On(_) | Self::Starting)
    }
}

/// The data folder and database (shared with the CLI on this device), and
/// the running shares, by stick.
pub struct AppState {
    pub data_dir: PathBuf,
    pub store: Arc<dyn Store>,
    pub sticks: tokio::sync::Mutex<HashMap<StickId, Sharing>>,
    /// The disks every shared stick reads, with their turns (ADR 0013).
    pub disks: Arc<DiskTurns>,
    /// Sticks being copied onto now (a drop): another drop waits.
    pub copying: std::sync::Mutex<std::collections::HashSet<StickId>>,
    /// The Local Network step was shown in this run (ADR 0020), remembered
    /// in the store once the device exists.
    pub local_network_explained: std::sync::atomic::AtomicBool,
    /// The start (sharing, receiving) waits for that step.
    pub start_waits: std::sync::atomic::AtomicBool,
}

impl AppState {
    pub fn open() -> Result<Self> {
        let data_dir = device::data_dir()?;
        let store = device::open_store(&data_dir)?;
        Ok(Self {
            data_dir,
            store,
            sticks: tokio::sync::Mutex::new(HashMap::new()),
            disks: Arc::new(DiskTurns::new(Arc::new(SystemTimer::default()))),
            copying: std::sync::Mutex::default(),
            local_network_explained: std::sync::atomic::AtomicBool::new(false),
            start_waits: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// The app's sticks, oldest first.
    pub fn app_sticks(&self) -> Result<Vec<OwnedStick>> {
        app_sticks(self.store.as_ref())
    }

    /// [`AppState::app_sticks`] off the async runtime (CLAUDE.md rule 7).
    pub async fn sticks_now(&self) -> Result<Vec<OwnedStick>> {
        let store = self.store.clone();
        tauri::async_runtime::spawn_blocking(move || app_sticks(store.as_ref())).await?
    }

    /// [`AppState::app_stick`] off the async runtime (CLAUDE.md rule 7).
    pub async fn stick_now(&self, id: &StickId) -> Result<OwnedStick> {
        let store = self.store.clone();
        let id = *id;
        tauri::async_runtime::spawn_blocking(move || {
            store
                .owned_stick(&id)?
                .filter(|stick| stick.kind == StickKind::App)
                .context("that stick is not on this device any more")
        })
        .await?
    }

    /// Records whether the user wants `id` shared, off the async runtime.
    pub async fn set_sharing_wanted(&self, id: &StickId, wanted: bool) -> Result<()> {
        let store = self.store.clone();
        let id = *id;
        tauri::async_runtime::spawn_blocking(move || Ok(store.set_sharing_wanted(&id, wanted)?))
            .await?
    }

    /// The app's stick `id`.
    pub fn app_stick(&self, id: &StickId) -> Result<OwnedStick> {
        self.store
            .owned_stick(id)?
            .filter(|stick| stick.kind == StickKind::App)
            .context("that stick is not on this device any more")
    }
}

/// The app's sticks (`StickKind::App`), oldest first: the one place that
/// says which sticks are the app's.
pub fn app_sticks(store: &dyn Store) -> Result<Vec<OwnedStick>> {
    Ok(store
        .owned_sticks()?
        .into_iter()
        .filter(|stick| stick.kind == StickKind::App)
        .collect())
}

/// A stick id as the UI passes it (64 hex digits).
pub fn stick_id(hex: &str) -> Result<StickId, String> {
    let not_one = || "not a stick id".to_owned();
    if hex.len() != 64 || !hex.is_ascii() {
        return Err(not_one());
    }
    let mut bytes = [0u8; 32];
    for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| not_one())?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| not_one())?;
    }
    StickId::from_bytes(&bytes).map_err(|_| not_one())
}

/// A stick id as the UI gets it.
pub fn stick_hex(id: &StickId) -> String {
    id.to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stick_id_goes_to_the_ui_and_back() {
        let id = ourstick_core::StickSigningKey::from_bytes(&[7; 32]).stick_id();
        assert_eq!(stick_id(&stick_hex(&id)), Ok(id));
        assert_eq!(stick_id(&stick_hex(&id).to_uppercase()), Ok(id));
        assert!(stick_id("zz").is_err());
        assert!(stick_id(&"ab".repeat(31)).is_err());
    }
}
