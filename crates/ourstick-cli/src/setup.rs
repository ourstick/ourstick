//! What every command needs: the data folder, the secret store and the
//! database.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use ourstick_core::secrets::{check_secret_store, KeyFileParams, SecretStore};
use ourstick_core::store::SecretBackend;
use ourstick_core::Store;
use ourstick_core::{device, locks};

use crate::secrets::{open_secrets, SecretsChoice, SystemKeyring, TerminalPassphrases};

pub struct Setup {
    /// With a key file, held until the program ends: a key file is read whole
    /// when opened and written whole on every change (`ourstick_core::locks`).
    pub key_file_lock: Option<locks::Lock>,
    pub data_dir: PathBuf,
    pub store: Arc<dyn Store>,
    pub secrets: Box<dyn SecretStore>,
}

/// This system's keyring (see [`SystemKeyring`]).
fn system_keyring() -> SystemKeyring<'static> {
    fn open() -> Result<Box<dyn SecretStore>> {
        Ok(device::open_system_keyring()?)
    }
    SystemKeyring {
        backend: device::system_keyring_backend(),
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

/// Opens everything for sharing, recording the device on first use; the keys
/// go where `choice` says, and stay there. May ask for a passphrase.
/// Blocking: call from `spawn_blocking`.
pub fn open(choice: &SecretsChoice) -> Result<Setup> {
    let ReaderSetup { data_dir, store } = open_for_reading()?;
    let device = store.device()?;
    let recorded = device.as_ref().map(|device| &device.secret_backend);
    // Before the key file is opened (and before any passphrase is asked
    // for): nothing to ask if another program uses it.
    let key_file = match recorded {
        Some(backend) => matches!(backend, SecretBackend::KeyFile(_)),
        None => matches!(choice, SecretsChoice::KeyFile(_)),
    };
    let key_file_lock = if key_file {
        Some(locks::key_file_lock(&data_dir)?)
    } else {
        None
    };
    let (secrets, backend) = open_secrets(
        recorded,
        choice,
        &system_keyring(),
        &TerminalPassphrases,
        KeyFileParams::default(),
    )?;
    check_secret_store(secrets.as_ref())?;
    // Only now, after any passphrase: one program records the device at a
    // time. If another recorded it meanwhile with other keys, this refuses.
    {
        let _setup = locks::setup_lock(&data_dir)?;
        device::open_device(store.as_ref(), &data_dir, secrets.as_ref(), backend)?;
    }
    Ok(Setup {
        key_file_lock,
        data_dir,
        store,
        secrets,
    })
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
