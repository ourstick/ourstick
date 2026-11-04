//! Where secrets live: the stick signing key, the StickKey, the node key, and a
//! reader's tickets. SQLite never holds them; it holds only a [`SecretName`].
//!
//! Every store must pass [`check_secret_store`] before use. A store that is not
//! durable, or that does not give back what was written, is an error, never a
//! silent fallback. See "Key storage" in `docs/protocol.md`.
//!
//! All methods block. From async code, call them inside `spawn_blocking`.

mod key_file;
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    windows,
    target_os = "android"
))]
mod keyring;
mod memory;
mod moving;

use std::fmt;

use rand::RngCore;

use crate::stick_id::StickId;

pub use key_file::{KeyFileParams, KeyFileStore};
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    windows,
    target_os = "android"
))]
pub use keyring::KeyringStore;
pub use memory::MemorySecretStore;
pub use moving::{
    all_there, copy_secrets, delete_old, finish_move, move_keys, secrets_to_move, MoveError, ToMove,
};

/// How long a store keeps what is written to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// Kept until deleted, across restarts and reboots.
    Durable,
    /// Lost at logout, reboot, or process exit, or unknown.
    Ephemeral,
}

/// The name of one secret in a [`SecretStore`]. It is not secret itself, and is
/// what SQLite stores in place of the secret.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum SecretName {
    /// The iroh node secret key of this device, used by the sticks made
    /// before ADR 0009.
    NodeKey,
    /// The iroh node secret key of one stick (ADR 0009).
    StickNodeKey(StickId),
    /// The owner's signing key for a stick.
    StickSigningKey(StickId),
    /// The symmetric key of a stick.
    StickKey(StickId),
    /// A ticket a reader holds for a stick.
    Ticket(StickId),
    /// A reader's own key for a stick it receives, which signs its reports
    /// to the owner (ADR 0015).
    ReaderKey(StickId),
    /// A short-lived entry written by [`check_secret_store`].
    HealthCheck([u8; 8]),
}

impl SecretName {
    /// The stable string form, e.g. `stick/<hex stick id>/signing-key`.
    pub fn as_key(&self) -> String {
        let hex = |stick_id: &StickId| data_encoding::HEXLOWER.encode(&stick_id.to_bytes());
        match self {
            Self::NodeKey => "node-key".to_owned(),
            Self::StickSigningKey(id) => format!("stick/{}/signing-key", hex(id)),
            Self::StickKey(id) => format!("stick/{}/stick-key", hex(id)),
            Self::StickNodeKey(id) => format!("stick/{}/node-key", hex(id)),
            Self::Ticket(id) => format!("stick/{}/ticket", hex(id)),
            Self::ReaderKey(id) => format!("stick/{}/reader-key", hex(id)),
            Self::HealthCheck(nonce) => {
                format!("health-check/{}", data_encoding::HEXLOWER.encode(nonce))
            }
        }
    }
}

impl fmt::Debug for SecretName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "SecretName({})", self.as_key())
    }
}

/// Stores secrets by name. Implementations must never log secret values.
pub trait SecretStore: Send + Sync {
    /// A short description for messages, e.g. "Secret Service" or "key file /path".
    fn describe(&self) -> String;

    fn durability(&self) -> Durability;

    /// Stores `secret` under `name`, replacing any previous value.
    fn put(&self, name: &SecretName, secret: &[u8]) -> Result<(), SecretError>;

    /// Returns the secret under `name`, or `None` if there is none.
    fn get(&self, name: &SecretName) -> Result<Option<Vec<u8>>, SecretError>;

    /// Deletes the secret under `name`. Deleting a missing secret is not an error.
    fn delete(&self, name: &SecretName) -> Result<(), SecretError>;
}

/// Why a secret could not be stored or read. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("no secure storage available ({store}): {reason}")]
    Unavailable { store: String, reason: String },
    #[error("access to {store} was denied or it is locked: {reason}")]
    AccessDenied { store: String, reason: String },
    #[error("you chose Deny, so OurStick could not read its keys in {store}")]
    UserDenied { store: String },
    #[error("{store} does not keep secrets across restarts, so keys would be lost")]
    NotDurable { store: String },
    #[error("{store} did not give back what was written to it: {reason}")]
    Unreliable { store: String, reason: String },
    #[error("wrong passphrase, or the key file is damaged")]
    WrongPassphraseOrDamaged,
    #[error("the passphrase must not be empty")]
    EmptyPassphrase,
    #[error("{path} can be read by other users (mode {mode:o}); run: chmod 600 {path}")]
    InsecurePermissions { path: String, mode: u32 },
    #[error("{path} is not an OurStick key file")]
    NotAKeyFile { path: String },
    #[error("{path} belongs to another user than its folder; OurStick will not read keys from it")]
    NotYours { path: String },
    #[error(
        "{path} was written by a newer version of OurStick (format {version}); update OurStick"
    )]
    UnsupportedKeyFileVersion { path: String, version: u8 },
    #[error("key file parameters are out of range: {0}")]
    InvalidKeyFileParams(String),
    #[error("{store} failed: {reason}")]
    Backend { store: String, reason: String },
    #[error("I/O error on {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Checks that `store` can be trusted with keys: it must be durable, and a test
/// entry written to it must read back identically and then delete cleanly.
///
/// Call this at every start, before the store holds anything that matters.
pub fn check_secret_store(store: &dyn SecretStore) -> Result<(), SecretError> {
    if store.durability() != Durability::Durable {
        return Err(SecretError::NotDurable {
            store: store.describe(),
        });
    }
    let mut nonce = [0u8; 8];
    let mut value = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    rand::rngs::OsRng.fill_bytes(&mut value);
    let name = SecretName::HealthCheck(nonce);
    let unreliable = |reason: &str| SecretError::Unreliable {
        store: store.describe(),
        reason: reason.to_owned(),
    };

    store.put(&name, &value)?;
    let read_back = store.get(&name);
    // Try to clean up even if the read failed.
    let deleted = store.delete(&name);
    match read_back? {
        Some(read) if read == value => {}
        Some(_) => return Err(unreliable("a test entry came back with different content")),
        None => {
            return Err(unreliable(
                "a test entry was written but could not be found",
            ))
        }
    }
    deleted?;
    if store.get(&name)?.is_some() {
        return Err(unreliable("a deleted test entry is still there"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
