//! Secrets in the system keyring: the freedesktop Secret Service (GNOME
//! Keyring, KWallet) over D-Bus on Linux, the login keychain on macOS, the
//! user's Credential Manager on Windows.
//!
//! Uses `keyring-core` with the platform's store chosen explicitly, so there
//! is no default or mock store to fall back to. Items are stored with service
//! `ourstick` and the [`SecretName`] as user.

use std::collections::HashMap;
use std::sync::Arc;

use keyring_core::{CredentialPersistence, CredentialStore, Error as KeyringError};

use super::{Durability, SecretError, SecretName, SecretStore};

const SERVICE: &str = "ourstick";

/// A [`SecretStore`] backed by the system keyring.
pub struct KeyringStore {
    store: Arc<CredentialStore>,
    name: &'static str,
    /// Store-specific settings for every entry.
    modifiers: &'static [(&'static str, &'static str)],
}

impl KeyringStore {
    /// Connects to the Secret Service on the session D-Bus.
    ///
    /// Fails with [`SecretError::Unavailable`] when there is no session bus or
    /// no Secret Service provider, e.g. over SSH or on a server.
    #[cfg(target_os = "linux")]
    pub fn secret_service() -> Result<Self, SecretError> {
        const NAME: &str = "Secret Service";
        let store = zbus_secret_service_keyring_store::Store::new().map_err(|error| {
            SecretError::Unavailable {
                store: NAME.to_owned(),
                reason: format!(
                    "{error}. Start and unlock a keyring (GNOME Keyring or KWallet), \
                     or use a key file instead"
                ),
            }
        })?;
        Ok(Self {
            store,
            name: NAME,
            modifiers: &[],
        })
    }

    /// Opens the user's login keychain.
    #[cfg(target_os = "macos")]
    pub fn keychain() -> Result<Self, SecretError> {
        const NAME: &str = "the macOS keychain";
        let store = apple_native_keyring_store::keychain::Store::new().map_err(|error| {
            SecretError::Unavailable {
                store: NAME.to_owned(),
                reason: error.to_string(),
            }
        })?;
        Ok(Self {
            store,
            name: NAME,
            modifiers: &[],
        })
    }

    /// Opens the user's Credential Manager.
    ///
    /// Entries are kept on this computer only (`CRED_PERSIST_LOCAL_MACHINE`):
    /// the default would let them roam with a domain profile to other
    /// computers, which would then serve with this device's node key.
    #[cfg(windows)]
    pub fn credential_manager() -> Result<Self, SecretError> {
        const NAME: &str = "the Windows Credential Manager";
        let store = windows_native_keyring_store::Store::new().map_err(|error| {
            SecretError::Unavailable {
                store: NAME.to_owned(),
                reason: error.to_string(),
            }
        })?;
        Ok(Self {
            store,
            name: NAME,
            modifiers: &[("persistence", "Local")],
        })
    }

    fn entry(&self, name: &SecretName) -> Result<keyring_core::Entry, SecretError> {
        let modifiers: HashMap<&str, &str> = self.modifiers.iter().copied().collect();
        self.store
            .build(
                SERVICE,
                &name.as_key(),
                (!modifiers.is_empty()).then_some(&modifiers),
            )
            .map_err(|error| to_secret_error(self.name, error))
    }
}

fn to_secret_error(store: &str, error: KeyringError) -> SecretError {
    let store = store.to_owned();
    match error {
        KeyringError::NoStorageAccess(reason) => SecretError::AccessDenied {
            store,
            reason: reason.to_string(),
        },
        KeyringError::NoDefaultStore => SecretError::Unavailable {
            store,
            reason: "no store was configured".to_owned(),
        },
        // The operating system's own error message; it carries no secret.
        KeyringError::PlatformFailure(error) => SecretError::Backend {
            store,
            reason: format!("platform failure: {error}"),
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

impl SecretStore for KeyringStore {
    fn describe(&self) -> String {
        self.name.to_owned()
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
            .map_err(|error| to_secret_error(self.name, error))
    }

    fn get(&self, name: &SecretName) -> Result<Option<Vec<u8>>, SecretError> {
        match self.entry(name)?.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(to_secret_error(self.name, error)),
        }
    }

    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(to_secret_error(self.name, error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::check_secret_store;

    /// Talks to the real keyring of the user running the tests, so it is
    /// ignored by default (CI has none). It writes one short-lived test entry
    /// under service `ourstick` and deletes it.
    /// Run with: `cargo test -p ourstick-core -- --ignored keyring`
    #[test]
    #[ignore = "needs an unlocked system keyring"]
    fn keyring_passes_startup_check_and_roundtrips() {
        #[cfg(target_os = "linux")]
        let store = KeyringStore::secret_service().expect("Secret Service available");
        #[cfg(target_os = "macos")]
        let store = KeyringStore::keychain().expect("keychain available");
        #[cfg(windows)]
        let store = KeyringStore::credential_manager().expect("Credential Manager available");
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
