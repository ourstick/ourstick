//! Where this device's keys are (ADR 0021, rule 2): the store its row names,
//! or, for a device not made yet, the app's own. The one place that decides;
//! every key is read and written through it. Blocking.

use anyhow::{Context, Result};
use ourstick_core::device;
use ourstick_core::secrets::SecretStore;
use ourstick_core::store::SecretBackend;
use ourstick_core::Store;

/// The store of this device's keys.
pub fn secrets(store: &dyn Store) -> Result<Box<dyn SecretStore>> {
    Ok(device::open_keyring(&backend(store)?)?)
}

/// The backend of this device's keys: its row's, or the app's for a new one.
pub fn backend(store: &dyn Store) -> Result<SecretBackend> {
    match store.device()? {
        Some(device) => Ok(device.secret_backend),
        None => for_a_new_device(cfg!(target_os = "macos"), cfg!(debug_assertions)),
    }
}

/// On macOS the data protection keychain, which never asks; a debug build,
/// which cannot carry the provisioning profile, keeps the login keychain
/// (decided at build time, never by a setting). Elsewhere the system's.
fn for_a_new_device(macos: bool, debug: bool) -> Result<SecretBackend> {
    if macos && !debug {
        Ok(SecretBackend::ProtectedKeychain)
    } else {
        device::system_keyring_backend().context("this system has no keyring OurStick can use yet")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_mac_device_keeps_its_keys_in_the_data_protection_keychain() {
        assert_eq!(
            for_a_new_device(true, false).unwrap(),
            SecretBackend::ProtectedKeychain
        );
    }

    #[test]
    fn a_device_keeps_the_backend_its_row_names() {
        let store = ourstick_core::MemoryStore::new();
        store
            .create_device(&ourstick_core::store::Device {
                node_id: None,
                secret_backend: SecretBackend::Keychain,
                created_at_ms: 0,
            })
            .unwrap();
        assert_eq!(backend(&store).unwrap(), SecretBackend::Keychain);
    }
}
