//! This device: its data folder and database, the sticks it owns with their
//! keys (a node key each since ADR 0009; the device's for older ones). Used
//! by every front end (CLI, app); all of it blocks, so call it from
//! `spawn_blocking`. The locks are in [`crate::locks`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rand::RngCore;

use crate::cipher::{CipherId, StickKey};
use crate::clock::{Clock, SystemClock};
use crate::secrets::{SecretError, SecretName, SecretStore};
use crate::stick_id::{StickId, StickSigningKey};
use crate::store::{Device, OwnedStick, SecretBackend, SqliteStore, StickKind, Store, StoreError};
use crate::transport::iroh::IrohConfig;

/// The cipher of every new stick.
const CIPHER: CipherId = CipherId::XChaCha20Poly1305;

/// Why this device could not be set up. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("cannot find your home folder ({0} is not set)")]
    NoHome(&'static str),
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
    #[error("this device keeps its keys in {recorded}, but this program uses {given}; keys made here would be where the database does not look for them. Use the program that uses {recorded}.")]
    OtherSecretStore { recorded: String, given: String },
    #[error("the network key of the stick \u{201c}{stick}\u{201d} is not the one recorded for it; OurStick will not use it (its links name another). Restore the right key.")]
    NodeKeyMismatch { stick: String },
    #[error("the stick \u{201c}{stick}\u{201d} uses this device's network key, but the database records none; the database is inconsistent")]
    NoDeviceNodeKey { stick: String },
    #[error(transparent)]
    Lock(#[from] crate::locks::LockError),
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

/// `$OURSTICK_DATA_DIR`, else on Windows `%LOCALAPPDATA%\OurStick`, else
/// `$XDG_DATA_HOME/ourstick`, else `$HOME/.local/share/ourstick`.
pub fn data_dir() -> Result<PathBuf, DeviceError> {
    data_dir_from(|name| std::env::var_os(name), cfg!(windows))
}

/// [`data_dir`] for the environment `variable` and the system `windows`.
fn data_dir_from(
    variable: impl Fn(&str) -> Option<std::ffi::OsString>,
    windows: bool,
) -> Result<PathBuf, DeviceError> {
    let set = |name: &str| variable(name).filter(|value| !value.is_empty());
    if let Some(dir) = set("OURSTICK_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if windows {
        // Local, not roaming: the keys' ids, the database and the blob cache
        // belong to this device.
        let local = set("LOCALAPPDATA").ok_or(DeviceError::NoHome("%LOCALAPPDATA%"))?;
        return Ok(PathBuf::from(local).join("OurStick"));
    }
    if let Some(dir) = set("XDG_DATA_HOME") {
        return Ok(PathBuf::from(dir).join("ourstick"));
    }
    let home = set("HOME").ok_or(DeviceError::NoHome("$HOME"))?;
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
/// the keychain on macOS, the Credential Manager on Windows, the Keystore on
/// Android; `None` where OurStick has no keyring backend yet.
pub fn system_keyring_backend() -> Option<SecretBackend> {
    if cfg!(target_os = "android") {
        Some(SecretBackend::AndroidKeystore)
    } else if cfg!(target_os = "linux") {
        Some(SecretBackend::SecretService)
    } else if cfg!(target_os = "macos") {
        Some(SecretBackend::Keychain)
    } else if cfg!(windows) {
        Some(SecretBackend::CredentialManager)
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
    #[cfg(windows)]
    return Ok(Box::new(crate::secrets::KeyringStore::credential_manager()?));
    #[cfg(target_os = "android")]
    return Ok(Box::new(crate::secrets::KeyringStore::android_keystore()?));
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        windows,
        target_os = "android"
    )))]
    Err(DeviceError::Secret(SecretError::Unavailable {
        store: "the system keyring".to_owned(),
        reason: "OurStick has no keyring backend for this system yet".to_owned(),
    }))
}

/// Opens the system store `backend` names (ADR 0021, rule 2): a device's
/// keys are read where its row says they are, never where this system keeps
/// them by default. A key file needs its passphrase, so it is opened by the
/// front end; a store of another system is refused.
pub fn open_keyring(backend: &SecretBackend) -> Result<Box<dyn SecretStore>, DeviceError> {
    match backend {
        #[cfg(target_os = "linux")]
        SecretBackend::SecretService => {
            Ok(Box::new(crate::secrets::KeyringStore::secret_service()?))
        }
        #[cfg(target_os = "macos")]
        SecretBackend::Keychain => Ok(Box::new(crate::secrets::KeyringStore::keychain()?)),
        #[cfg(target_os = "macos")]
        SecretBackend::ProtectedKeychain => {
            Ok(Box::new(crate::secrets::KeyringStore::protected_keychain()?))
        }
        #[cfg(windows)]
        SecretBackend::CredentialManager => {
            Ok(Box::new(crate::secrets::KeyringStore::credential_manager()?))
        }
        #[cfg(target_os = "android")]
        SecretBackend::AndroidKeystore => {
            Ok(Box::new(crate::secrets::KeyringStore::android_keystore()?))
        }
        other => Err(DeviceError::Secret(SecretError::Unavailable {
            store: backend_name(other),
            reason: "this system does not have it, or it needs a passphrase".to_owned(),
        })),
    }
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

/// Records this device on first use, with `backend` as where its keys live;
/// later, checks that `backend` is the one recorded, so no key is ever put
/// where the database does not look for it. A device made now has no node
/// key of its own: each stick gets one (ADR 0009). A device made before keeps
/// its node key (rule 9), loaded only for its older sticks. Take
/// [`crate::locks::setup_lock`] first: two programs must not record the
/// device at once.
pub fn open_device(
    store: &dyn Store,
    data_dir: &Path,
    secrets: &dyn SecretStore,
    backend: SecretBackend,
) -> Result<(), DeviceError> {
    if let Some(device) = store.device()? {
        if device.secret_backend != backend {
            return Err(DeviceError::OtherSecretStore {
                recorded: backend_name(&device.secret_backend),
                given: backend_name(&backend),
            });
        }
        return Ok(());
    }
    // A key without its database: the user's to sort out, never replaced.
    if secrets.get(&SecretName::NodeKey)?.is_some() {
        return Err(DeviceError::NodeKeyExists {
            store: secrets.describe(),
            database: data_dir.join("state.sqlite"),
        });
    }
    store.create_device(&Device {
        node_id: None,
        secret_backend: backend,
        created_at_ms: SystemClock.now_ms(),
    })?;
    Ok(())
}

/// How a backend is named in messages.
pub fn backend_name(backend: &SecretBackend) -> String {
    match backend {
        SecretBackend::SecretService => "the Secret Service".to_owned(),
        SecretBackend::Keychain => "the macOS keychain".to_owned(),
        SecretBackend::ProtectedKeychain => "the macOS data protection keychain".to_owned(),
        SecretBackend::CredentialManager => "the Windows Credential Manager".to_owned(),
        SecretBackend::AndroidKeystore => "the Android Keystore".to_owned(),
        SecretBackend::KeyFile(path) => format!("the key file {}", path.display()),
    }
}

/// An owned stick with its keys.
pub struct StickKeys {
    pub stick: OwnedStick,
    pub signing_key: StickSigningKey,
    pub stick_key: StickKey,
    /// The iroh node key it is shared with: its own, or the device's for a
    /// stick made before ADR 0009.
    pub node_key: [u8; 32],
}

/// Loads the keys of a stick this device owns. Its node key must be the one
/// the database records ("who" before "what": a key that names another node
/// would serve links nobody has).
pub fn load_stick(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    stick: OwnedStick,
) -> Result<StickKeys, DeviceError> {
    let signing = load_key(secrets, &SecretName::StickSigningKey(stick.stick_id))?;
    let stick_key = load_key(secrets, &SecretName::StickKey(stick.stick_id))?;
    let (name, recorded) = match stick.node_id {
        Some(node_id) => (SecretName::StickNodeKey(stick.stick_id), node_id),
        None => {
            let device_node = store.device()?.and_then(|device| device.node_id);
            let Some(node_id) = device_node else {
                return Err(DeviceError::NoDeviceNodeKey {
                    stick: stick.name.clone(),
                });
            };
            (SecretName::NodeKey, node_id)
        }
    };
    let node_key = load_key(secrets, &name)?;
    if IrohConfig::node_id_of(&node_key) != recorded {
        return Err(DeviceError::NodeKeyMismatch {
            stick: stick.name.clone(),
        });
    }
    Ok(StickKeys {
        stick,
        signing_key: StickSigningKey::from_bytes(&signing),
        stick_key: StickKey::from_bytes(stick_key),
        node_key,
    })
}

/// Makes a new stick for `root` with new keys, its own node key among them,
/// listed by the program `kind` (the app's is a drive), and on macOS the
/// disk image it lives in, recorded in the same write as its row (so no
/// crash leaves a stick without its image). The keys are stored first: a
/// stick in the database always has its keys, and keys left by a crash
/// before the row belong to no stick and are never used. Take
/// [`crate::locks::setup_lock`] first.
pub fn create_stick(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    root: &Path,
    name: &str,
    kind: StickKind,
    image_path: Option<&Path>,
) -> Result<StickKeys, DeviceError> {
    let signing_key = StickSigningKey::generate();
    let made = OwnedStick {
        kind,
        image_path: image_path.map(Path::to_path_buf),
        ..OwnedStick::new(
            signing_key.stick_id(),
            name,
            root,
            CIPHER,
            SystemClock.now_ms(),
        )
    };
    create(store, secrets, signing_key, made)
}

/// The folder of a stick with no folder of its own (ADR 0028 §7):
/// `<data>/sent/<stick id hex>`, by id so that a rename never moves it.
pub fn sent_folder(data_dir: &Path, stick_id: &StickId) -> PathBuf {
    let hex = data_encoding::HEXLOWER.encode(&stick_id.to_bytes());
    data_dir.join("sent").join(hex)
}

/// Makes a new app stick of ADR 0028: no folder of its own and no drive,
/// only what is sent from where it is (linked entries). Its root is an
/// empty folder, [`sent_folder`], made before its row. Take
/// [`crate::locks::setup_lock`] first.
pub fn create_sending_stick(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    name: &str,
) -> Result<StickKeys, DeviceError> {
    let signing_key = StickSigningKey::generate();
    let root = sent_folder(data_dir, &signing_key.stick_id());
    std::fs::create_dir_all(&root).map_err(io(&root))?;
    let made = OwnedStick {
        kind: StickKind::App,
        has_own_folder: false,
        ..OwnedStick::new(
            signing_key.stick_id(),
            name,
            &root,
            CIPHER,
            SystemClock.now_ms(),
        )
    };
    create(store, secrets, signing_key, made)
}

/// Stores `made`'s keys, then its row with its own node key.
fn create(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    signing_key: StickSigningKey,
    made: OwnedStick,
) -> Result<StickKeys, DeviceError> {
    let stick_key = StickKey::generate();
    let node_key = random_key();
    let stick_id = signing_key.stick_id();
    secrets.put(
        &SecretName::StickSigningKey(stick_id),
        &signing_key.to_bytes(),
    )?;
    secrets.put(&SecretName::StickKey(stick_id), stick_key.as_bytes())?;
    secrets.put(&SecretName::StickNodeKey(stick_id), &node_key)?;
    let stick = OwnedStick {
        node_id: Some(IrohConfig::node_id_of(&node_key)),
        ..made
    };
    store.add_owned_stick(&stick)?;
    Ok(StickKeys {
        stick,
        signing_key,
        stick_key,
        node_key,
    })
}

#[cfg(test)]
mod tests;
