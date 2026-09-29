//! This device: its data folder and database, its node key, the sticks it
//! owns with their keys, and the lock held while it shares. Used by every
//! front end (CLI, app); all of it blocks, so call it from `spawn_blocking`.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rand::RngCore;

use crate::cipher::{CipherId, StickKey};
use crate::clock::{Clock, SystemClock};
use crate::secrets::{SecretError, SecretName, SecretStore};
use crate::stick_id::StickSigningKey;
use crate::store::{Device, OwnedStick, SecretBackend, SqliteStore, Store, StoreError};
use crate::transport::iroh::IrohConfig;

/// The cipher of every new stick.
const CIPHER: CipherId = CipherId::XChaCha20Poly1305;

/// Why this device could not be set up. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("cannot find your home folder ($HOME is not set)")]
    NoHome,
    #[error("I/O error on {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the key {name} is missing from {store}. It was created earlier and is needed by tickets you already shared; OurStick will not replace it. Restore it (e.g. unlock or restore your keyring) and try again.")]
    KeyMissing { name: String, store: String },
    #[error("the key {name} in {store} is damaged")]
    KeyDamaged { name: String, store: String },
    #[error("{store} already holds an OurStick node key, but the database at {} is new. Restore the database or remove the old key yourself; OurStick will not overwrite it.", .database.display())]
    NodeKeyExists { store: String, database: PathBuf },
    #[error("OurStick is already sharing on this device (another `ourstick share` or the app); stop it first")]
    AlreadySharing,
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Secret(#[from] SecretError),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> DeviceError + '_ {
    move |source| DeviceError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// `$OURSTICK_DATA_DIR`, else `$XDG_DATA_HOME/ourstick`, else
/// `$HOME/.local/share/ourstick`.
pub fn data_dir() -> Result<PathBuf, DeviceError> {
    if let Some(dir) = std::env::var_os("OURSTICK_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir).join("ourstick"));
    }
    let home = std::env::var_os("HOME").ok_or(DeviceError::NoHome)?;
    Ok(PathBuf::from(home).join(".local/share/ourstick"))
}

fn create_private_dir(dir: &Path) -> Result<(), DeviceError> {
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(io(dir))?;
    }
    Ok(())
}

/// Creates the data folder (private to the user) if needed and opens the
/// database in it.
pub fn open_store(data_dir: &Path) -> Result<Arc<dyn Store>, DeviceError> {
    create_private_dir(data_dir)?;
    Ok(Arc::new(SqliteStore::open(data_dir.join("state.sqlite"))?))
}

/// Where this system keeps secrets by default: the Secret Service on Linux,
/// the keychain on macOS; `None` where OurStick has no keyring backend yet.
pub fn system_keyring_backend() -> Option<SecretBackend> {
    if cfg!(target_os = "linux") {
        Some(SecretBackend::SecretService)
    } else if cfg!(target_os = "macos") {
        Some(SecretBackend::Keychain)
    } else {
        None
    }
}

/// Opens this system's keyring (see [`system_keyring_backend`]).
pub fn open_system_keyring() -> Result<Box<dyn SecretStore>, DeviceError> {
    #[cfg(target_os = "linux")]
    return Ok(Box::new(crate::secrets::KeyringStore::secret_service()?));
    #[cfg(target_os = "macos")]
    return Ok(Box::new(crate::secrets::KeyringStore::keychain()?));
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Err(DeviceError::Secret(SecretError::Unavailable {
        store: "the system keyring".to_owned(),
        reason: "OurStick has no keyring backend for this system yet".to_owned(),
    }))
}

/// 32 random bytes, for a node key.
pub fn random_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    key
}

/// Reads a 32-byte key. A missing key is an error, never a reason to make a
/// new one (CLAUDE.md rule 9).
pub fn load_key(secrets: &dyn SecretStore, name: &SecretName) -> Result<[u8; 32], DeviceError> {
    let Some(bytes) = secrets.get(name)? else {
        return Err(DeviceError::KeyMissing {
            name: format!("{name:?}"),
            store: secrets.describe(),
        });
    };
    bytes.try_into().map_err(|_| DeviceError::KeyDamaged {
        name: format!("{name:?}"),
        store: secrets.describe(),
    })
}

/// This device's node key: loaded, or on first use created and recorded with
/// `backend` as where the keys live. Never replaces a key that exists.
pub fn open_device(
    store: &dyn Store,
    data_dir: &Path,
    secrets: &dyn SecretStore,
    backend: SecretBackend,
) -> Result<[u8; 32], DeviceError> {
    if store.device()?.is_some() {
        return load_key(secrets, &SecretName::NodeKey);
    }
    if secrets.get(&SecretName::NodeKey)?.is_some() {
        return Err(DeviceError::NodeKeyExists {
            store: secrets.describe(),
            database: data_dir.join("state.sqlite"),
        });
    }
    let key = random_key();
    secrets.put(&SecretName::NodeKey, &key)?;
    store.create_device(&Device {
        node_id: IrohConfig::node_id_of(&key),
        secret_backend: backend,
        created_at_ms: SystemClock.now_ms(),
    })?;
    Ok(key)
}

/// An owned stick with its keys.
pub struct StickKeys {
    pub stick: OwnedStick,
    pub signing_key: StickSigningKey,
    pub stick_key: StickKey,
}

/// Loads the keys of a stick this device owns.
pub fn load_stick(secrets: &dyn SecretStore, stick: OwnedStick) -> Result<StickKeys, DeviceError> {
    let signing = load_key(secrets, &SecretName::StickSigningKey(stick.stick_id))?;
    let stick_key = load_key(secrets, &SecretName::StickKey(stick.stick_id))?;
    Ok(StickKeys {
        stick,
        signing_key: StickSigningKey::from_bytes(&signing),
        stick_key: StickKey::from_bytes(stick_key),
    })
}

/// Makes a new stick for `root` with new keys; `drive_size` for a drive (the
/// app's stick). The keys are stored first: a stick in the database always
/// has its keys.
pub fn create_stick(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    root: &Path,
    name: &str,
    drive_size: Option<u64>,
) -> Result<StickKeys, DeviceError> {
    let signing_key = StickSigningKey::generate();
    let stick_key = StickKey::generate();
    let stick_id = signing_key.stick_id();
    secrets.put(
        &SecretName::StickSigningKey(stick_id),
        &signing_key.to_bytes(),
    )?;
    secrets.put(&SecretName::StickKey(stick_id), stick_key.as_bytes())?;
    let stick = OwnedStick {
        drive_size,
        ..OwnedStick::new(stick_id, name, root, CIPHER, SystemClock.now_ms())
    };
    store.add_owned_stick(&stick)?;
    Ok(StickKeys {
        stick,
        signing_key,
        stick_key,
    })
}

/// Held while this device shares: only one front end may serve its sticks at
/// a time, since they would share one node identity. Released when dropped
/// (and by the system if the process dies).
#[derive(Debug)]
pub struct ShareLock {
    _file: File,
}

impl ShareLock {
    pub fn acquire(data_dir: &Path) -> Result<Self, DeviceError> {
        let path = data_dir.join("share.lock");
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(io(&path))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(DeviceError::AlreadySharing),
            Err(std::fs::TryLockError::Error(source)) => Err(DeviceError::Io { path, source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecretStore;
    use crate::store::MemoryStore;

    #[test]
    fn the_node_key_is_made_once_and_then_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new();
        let secrets = MemorySecretStore::new();
        let first =
            open_device(&store, dir.path(), &secrets, SecretBackend::SecretService).unwrap();
        let again =
            open_device(&store, dir.path(), &secrets, SecretBackend::SecretService).unwrap();
        assert_eq!(first, again);
        assert_eq!(
            store.device().unwrap().unwrap().node_id,
            IrohConfig::node_id_of(&first)
        );
    }

    #[test]
    fn an_old_node_key_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecretStore::new();
        secrets.put(&SecretName::NodeKey, &[7; 32]).unwrap();
        let result = open_device(
            &MemoryStore::new(),
            dir.path(),
            &secrets,
            SecretBackend::SecretService,
        );
        assert!(matches!(result, Err(DeviceError::NodeKeyExists { .. })));
        assert_eq!(
            secrets.get(&SecretName::NodeKey).unwrap(),
            Some(vec![7; 32])
        );
    }

    #[test]
    fn a_created_stick_loads_with_the_same_keys() {
        let store = MemoryStore::new();
        let secrets = MemorySecretStore::new();
        let made = create_stick(&store, &secrets, Path::new("/tmp/s"), "s", Some(10)).unwrap();
        assert_eq!(made.stick.drive_size, Some(10));
        let stick = store.owned_sticks().unwrap().remove(0);
        let loaded = load_stick(&secrets, stick).unwrap();
        assert_eq!(loaded.signing_key.to_bytes(), made.signing_key.to_bytes());
        assert_eq!(loaded.stick_key.as_bytes(), made.stick_key.as_bytes());

        // A missing key is an error, never replaced.
        secrets
            .delete(&SecretName::StickKey(made.stick.stick_id))
            .unwrap();
        let stick = store.owned_sticks().unwrap().remove(0);
        assert!(matches!(
            load_stick(&secrets, stick),
            Err(DeviceError::KeyMissing { .. })
        ));
    }

    #[test]
    fn only_one_may_share_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let first = ShareLock::acquire(dir.path()).unwrap();
        assert!(matches!(
            ShareLock::acquire(dir.path()),
            Err(DeviceError::AlreadySharing)
        ));
        drop(first);
        ShareLock::acquire(dir.path()).unwrap();
    }
}
