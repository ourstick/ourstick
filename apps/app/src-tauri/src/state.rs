//! What the app keeps while it runs.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use ourstick_core::device;
use ourstick_core::Store;

/// The data folder and database, shared with the CLI on this device.
pub struct AppState {
    pub data_dir: PathBuf,
    pub store: Arc<dyn Store>,
}

impl AppState {
    pub fn open() -> Result<Self> {
        let data_dir = device::data_dir()?;
        let store = device::open_store(&data_dir)?;
        Ok(Self { data_dir, store })
    }

    /// The app's stick: the owned stick that is a drive, if there is one yet.
    pub fn drive_stick(&self) -> Result<Option<ourstick_core::OwnedStick>> {
        Ok(self
            .store
            .owned_sticks()?
            .into_iter()
            .find(|stick| stick.drive_size.is_some()))
    }
}
