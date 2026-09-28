//! Secrets in the freedesktop Secret Service (GNOME Keyring, KWallet), over D-Bus.
//!
//! Uses `keyring-core` with the zbus Secret Service store chosen explicitly, so
//! there is no default or mock store to fall back to. Items are stored with
//! service `ourstick` and the [`SecretName`] as user.

use std::sync::Arc;

use keyring_core::api::CredentialStoreApi;
use keyring_core::{CredentialPersistence, Error as KeyringError};
use zbus_secret_service_keyring_store::Store;

use super::{Durability, SecretError, SecretName, SecretStore};

const SERVICE: &str = "ourstick";
const STORE_NAME: &str = "Secret Service";

/// A [`SecretStore`] backed by the user's Secret Service.
pub struct SecretServiceStore {
    store: Arc<Store>,
}

impl SecretServiceStore {
    /// Connects to the Secret Service on the session D-Bus.
    ///
    /// Fails with [`SecretError::Unavailable`] when there is no session bus or
    /// no Secret Service provider, e.g. over SSH or on a server.
    pub fn connect() -> Result<Self, SecretError> {
        let store = Store::new().map_err(|error| SecretError::Unavailable {
            store: STORE_NAME.to_owned(),
            reason: format!(
                "{error}. Start and unlock a keyring (GNOME Keyring or KWallet), \
                 or use a key file instead"
            ),
        })?;
        Ok(Self { store })
    }

    fn entry(&self, name: &SecretName) -> Result<keyring_core::Entry, SecretError> {
        self.store
            .build(SERVICE, &name.as_key(), None)
            .map_err(to_secret_error)
    }
}

fn to_secret_error(error: KeyringError) -> SecretError {
    let store = STORE_NAME.to_owned();
    match error {
        KeyringError::NoStorageAccess(reason) => SecretError::AccessDenied {
            store,
            reason: reason.to_string(),
        },
        KeyringError::NoDefaultStore => SecretError::Unavailable {
            store,
            reason: "no store was configured".to_owned(),
        },
        // Other variants may carry the stored bytes (e.g. BadEncoding), so only
        // the kind of error is reported, never its payload.
        other => SecretError::Backend {
            store,
            reason: error_kind(&other).to_owned(),
        },
    }
}

fn error_kind(error: &KeyringError) -> &'static str {
    match error {
        KeyringError::PlatformFailure(_) => "platform failure",
        KeyringError::NoEntry => "entry not found",
        KeyringError::BadEncoding(_) => "stored value has a bad encoding",
        KeyringError::BadDataFormat(..) => "stored value has a bad format",
        KeyringError::BadStoreFormat(_) => "store has a bad format",
        KeyringError::TooLong(..) => "value too long",
        KeyringError::Invalid(..) => "invalid argument",
        KeyringError::Ambiguous(_) => "more than one matching entry",
        KeyringError::NotSupportedByStore(_) => "not supported by the store",
        _ => "unknown error",
    }
}

impl SecretStore for SecretServiceStore {
    fn describe(&self) -> String {
        STORE_NAME.to_owned()
    }

    fn durability(&self) -> Durability {
        match self.store.persistence() {
            CredentialPersistence::UntilDelete => Durability::Durable,
            _ => Durability::Ephemeral,
        }
    }

    fn put(&self, name: &SecretName, secret: &[u8]) -> Result<(), SecretError> {
        self.entry(name)?
            .set_secret(secret)
            .map_err(to_secret_error)
    }

    fn get(&self, name: &SecretName) -> Result<Option<Vec<u8>>, SecretError> {
        match self.entry(name)?.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(to_secret_error(error)),
        }
    }

    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(to_secret_error(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::check_secret_store;

    /// Talks to the real Secret Service of the user running the tests, so it is
    /// ignored by default (CI has none). It writes one short-lived test entry
    /// under service `ourstick` and deletes it.
    /// Run with: `cargo test -p ourstick-core -- --ignored secret_service`
    #[test]
    #[ignore = "needs a running, unlocked Secret Service"]
    fn secret_service_passes_startup_check_and_roundtrips() {
        let store = SecretServiceStore::connect().expect("Secret Service available");
        check_secret_store(&store).unwrap();

        let name = SecretName::HealthCheck(*b"roundtrp");
        store.put(&name, b"\x00\xffbinary").unwrap();
        assert_eq!(
            store.get(&name).unwrap().as_deref(),
            Some(&b"\x00\xffbinary"[..])
        );
        store.delete(&name).unwrap();
        assert_eq!(store.get(&name).unwrap(), None);
        store.delete(&name).unwrap();
    }
}
