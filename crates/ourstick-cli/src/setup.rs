//! What every command needs: the data folder, the secret store, the database,
//! and this device's node key.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use ourstick_core::secrets::{check_secret_store, SecretName, SecretStore};
use ourstick_core::transport::iroh::IrohConfig;
use ourstick_core::{Device, SecretBackend, SqliteStore, Store};
use rand::RngCore;

pub struct Setup {
    pub data_dir: PathBuf,
    pub store: Arc<dyn Store>,
    pub secrets: Box<dyn SecretStore>,
    pub node_key: [u8; 32],
}

/// `$OURSTICK_DATA_DIR`, else `$XDG_DATA_HOME/ourstick`, else
/// `$HOME/.local/share/ourstick`.
fn data_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("OURSTICK_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir).join("ourstick"));
    }
    let home =
        std::env::var_os("HOME").context("cannot find your home folder ($HOME is not set)")?;
    Ok(PathBuf::from(home).join(".local/share/ourstick"))
}

fn create_private_dir(dir: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("cannot make {} private", dir.display()))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn secret_store() -> Result<Box<dyn SecretStore>> {
    let store = ourstick_core::secrets::SecretServiceStore::connect()?;
    Ok(Box::new(store))
}

#[cfg(not(target_os = "linux"))]
fn secret_store() -> Result<Box<dyn SecretStore>> {
    bail!("this version of OurStick keeps keys only in the Linux Secret Service")
}

pub fn random_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    key
}

/// Reads a 32-byte key from the secret store. A missing key is an error, never
/// a reason to make a new one (CLAUDE.md rule 9).
pub fn load_key(secrets: &dyn SecretStore, name: &SecretName) -> Result<[u8; 32]> {
    let Some(bytes) = secrets.get(name)? else {
        bail!(
            "the key {name:?} is missing from {}. It was created earlier and is needed by \
             tickets you already shared; OurStick will not replace it. Restore it (e.g. unlock \
             or restore your keyring) and try again.",
            secrets.describe()
        );
    };
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("the key {name:?} in {} is damaged", secrets.describe()))
}

/// Opens everything, creating the device and its node key on first use.
/// Blocking: call from `spawn_blocking`.
/// What a reader needs: the data folder and the database, no secret store.
/// Works on every platform.
pub struct ReaderSetup {
    pub data_dir: PathBuf,
    pub store: Arc<dyn Store>,
}

/// Opens the data folder and database only. Blocking: call from `spawn_blocking`.
pub fn open_for_reading() -> Result<ReaderSetup> {
    let data_dir = data_dir()?;
    create_private_dir(&data_dir)?;
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open(data_dir.join("state.sqlite"))?);
    Ok(ReaderSetup { data_dir, store })
}

pub fn open() -> Result<Setup> {
    let ReaderSetup { data_dir, store } = open_for_reading()?;
    let secrets = secret_store()?;
    check_secret_store(secrets.as_ref())?;

    let node_key = match store.device()? {
        Some(device) => {
            if device.secret_backend != SecretBackend::SecretService {
                bail!("this device keeps its keys in a key file, which this version of the CLI cannot open yet");
            }
            load_key(secrets.as_ref(), &SecretName::NodeKey)?
        }
        None => {
            if secrets.get(&SecretName::NodeKey)?.is_some() {
                bail!(
                    "{} already holds an OurStick node key, but the database at {} is new. \
                     Restore the database or remove the old key yourself; OurStick will not \
                     overwrite it.",
                    secrets.describe(),
                    data_dir.display()
                );
            }
            let key = random_key();
            secrets.put(&SecretName::NodeKey, &key)?;
            store.create_device(&Device {
                node_id: IrohConfig::node_id_of(&key),
                secret_backend: SecretBackend::SecretService,
                created_at_ms: now_ms(),
            })?;
            key
        }
    };
    Ok(Setup {
        data_dir,
        store,
        secrets,
        node_key,
    })
}

pub fn now_ms() -> u64 {
    use ourstick_core::clock::{Clock, SystemClock};
    SystemClock.now_ms()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
