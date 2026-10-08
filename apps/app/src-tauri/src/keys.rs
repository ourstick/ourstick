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

/// How many keys a move to the data protection keychain would read (macOS
/// asks once for each), if this device's keys are still in the login
/// keychain and this build can keep them there (ADR 0021, rule 3).
pub fn move_due(store: &dyn Store) -> Result<Option<usize>> {
    let from_login = matches!(
        store.device()?.map(|device| device.secret_backend),
        Some(SecretBackend::Keychain)
    );
    if !from_login
        || for_a_new_device(cfg!(target_os = "macos"), cfg!(debug_assertions))?
            != SecretBackend::ProtectedKeychain
    {
        return Ok(None);
    }
    let to_move = ourstick_core::secrets::secrets_to_move(store)?;
    Ok(Some(to_move.required.len() + to_move.optional.len()))
}

/// Moves the keys (the user said Continue). Blocking; under the setup lock.
pub fn move_now(store: &dyn Store, data_dir: &std::path::Path) -> Result<usize> {
    let _setup = ourstick_core::locks::setup_lock(data_dir)?;
    let from = device::open_keyring(&SecretBackend::Keychain)?;
    let to = device::open_keyring(&SecretBackend::ProtectedKeychain)?;
    ourstick_core::check_secret_store(to.as_ref())?;
    Ok(ourstick_core::secrets::move_keys(
        store,
        from.as_ref(),
        to.as_ref(),
    )?)
}

/// At a start after a move: the login keychain's old copies go, once every
/// key reads from the new store. A failure only waits for the next start.
pub fn finish_move(store: &dyn Store) {
    if !store.old_keys_left().unwrap_or(false) {
        return;
    }
    let finished = (|| -> Result<bool> {
        let from = device::open_keyring(&SecretBackend::Keychain)?;
        let to = device::open_keyring(&SecretBackend::ProtectedKeychain)?;
        Ok(ourstick_core::secrets::finish_move(
            store,
            from.as_ref(),
            to.as_ref(),
        )?)
    })();
    if let Err(error) = finished {
        tracing::warn!(%error, "the old copies of the moved keys stay for now");
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
    fn a_device_already_in_the_data_protection_keychain_has_nothing_to_move() {
        let store = ourstick_core::MemoryStore::new();
        assert_eq!(move_due(&store).unwrap(), None, "no device");
        store
            .create_device(&ourstick_core::store::Device {
                node_id: None,
                secret_backend: SecretBackend::ProtectedKeychain,
                created_at_ms: 0,
            })
            .unwrap();
        assert_eq!(move_due(&store).unwrap(), None);
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
