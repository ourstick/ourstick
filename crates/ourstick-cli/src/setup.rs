//! What every command needs: the data folder, the secret store, the database,
//! and this device's node key.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use ourstick_core::device;
use ourstick_core::secrets::{check_secret_store, KeyFileParams, SecretStore};
use ourstick_core::{SecretBackend, Store};

use crate::secrets::{open_secrets, SecretsChoice, SystemKeyring, TerminalPassphrases};

pub struct Setup {
    /// Held while sharing: one sharer per device.
    pub lock: device::ShareLock,
    pub data_dir: PathBuf,
    pub store: Arc<dyn Store>,
    pub secrets: Box<dyn SecretStore>,
    pub node_key: [u8; 32],
}

/// This system's keyring (see [`SystemKeyring`]).
fn system_keyring() -> SystemKeyring<'static> {
    #[cfg(target_os = "linux")]
    fn open() -> Result<Box<dyn SecretStore>> {
        Ok(Box::new(
            ourstick_core::secrets::KeyringStore::secret_service()?,
        ))
    }
    #[cfg(target_os = "macos")]
    fn open() -> Result<Box<dyn SecretStore>> {
        Ok(Box::new(ourstick_core::secrets::KeyringStore::keychain()?))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn open() -> Result<Box<dyn SecretStore>> {
        anyhow::bail!("on this system OurStick can use only a key file for now")
    }
    let backend = if cfg!(target_os = "linux") {
        Some(SecretBackend::SecretService)
    } else if cfg!(target_os = "macos") {
        Some(SecretBackend::Keychain)
    } else {
        None
    };
    SystemKeyring {
        backend,
        open: &open,
    }
}

/// What a reader needs: the data folder and the database, no secret store.
/// Works on every platform.
pub struct ReaderSetup {
    pub data_dir: PathBuf,
    pub store: Arc<dyn Store>,
}

/// Opens the data folder and database only. Blocking: call from `spawn_blocking`.
pub fn open_for_reading() -> Result<ReaderSetup> {
    let data_dir = device::data_dir()?;
    let store = device::open_store(&data_dir)?;
    Ok(ReaderSetup { data_dir, store })
}

/// Opens everything for sharing, creating the device and its node key on first
/// use, and takes the device's share lock; the
/// keys go where `choice` says, and stay there. May ask for a passphrase.
/// Blocking: call from `spawn_blocking`.
pub fn open(choice: &SecretsChoice) -> Result<Setup> {
    let ReaderSetup { data_dir, store } = open_for_reading()?;
    // Before any passphrase is asked for: nothing to ask if another sharer
    // runs.
    let lock = device::ShareLock::acquire(&data_dir)?;
    let device = store.device()?;
    let (secrets, backend) = open_secrets(
        device.as_ref().map(|device| &device.secret_backend),
        choice,
        &system_keyring(),
        &TerminalPassphrases,
        KeyFileParams::default(),
    )?;
    check_secret_store(secrets.as_ref())?;
    let node_key = device::open_device(store.as_ref(), &data_dir, secrets.as_ref(), backend)?;
    Ok(Setup {
        lock,
        data_dir,
        store,
        secrets,
        node_key,
    })
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
