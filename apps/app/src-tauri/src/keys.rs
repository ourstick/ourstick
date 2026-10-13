//! Where this device's keys are (ADR 0021, rule 2): the store its row names,
//! or, for a device not made yet, the app's own. The one place that decides;
//! every key is read and written through it. Blocking.

use anyhow::{Context, Result};
use ourstick_core::device;
use ourstick_core::secrets::{Durability, SecretError, SecretName, SecretStore};
use ourstick_core::store::{SecretBackend, StickKind};
use ourstick_core::Store;

/// Whether the keys' move waits for its step or runs (ADR 0021): until it
/// is done or put off, nothing but the step reads or writes a key.
pub fn waiting(state: &crate::state::AppState) -> bool {
    state.keys_wait.load(std::sync::atomic::Ordering::Relaxed)
}

/// The store of this device's keys. While the login keychain still holds
/// the copies of moved keys, a key deleted goes from both.
pub fn secrets(store: &dyn Store) -> Result<Box<dyn SecretStore>> {
    let backend = backend(store)?;
    let secrets = device::open_keyring(&backend)?;
    if backend == SecretBackend::ProtectedKeychain && store.old_keys_left()? {
        let old = device::open_keyring(&SecretBackend::Keychain)?;
        return Ok(Box::new(DeletedFromBoth { new: secrets, old }));
    }
    Ok(secrets)
}

/// The backend of this device's keys: its row's; with no row, the login
/// keychain if it already received sticks there (before ADR 0021 a device
/// that only received had no row), else the app's for a new device.
pub fn backend(store: &dyn Store) -> Result<SecretBackend> {
    match store.device()? {
        // Only a build with its profile reads them: say so, not -34018.
        Some(device)
            if device.secret_backend == SecretBackend::ProtectedKeychain
                && !carries_its_profile() =>
        {
            anyhow::bail!(
                "this Mac's keys are in the data protection keychain, which only the \
                 signed OurStick app reads; this build cannot (run it with its own \
                 OURSTICK_DATA_DIR)"
            )
        }
        Some(device) => Ok(device.secret_backend),
        None if received_without_a_row(store)? => device::system_keyring_backend()
            .context("this system has no keyring OurStick can use yet"),
        None => for_a_new_device(cfg!(target_os = "macos"), carries_its_profile()),
    }
}

/// Received (or waited for, ADR 0025) by the app before the device row.
fn received_without_a_row(store: &dyn Store) -> Result<bool> {
    Ok(store.device()?.is_none()
        && (store
            .joined_sticks()?
            .iter()
            .any(|joined| joined.kind == StickKind::App)
            || !store.waiting_links()?.is_empty()))
}

/// On macOS the data protection keychain, which never asks, if this build
/// carries its provisioning profile (`make-dmg.sh`); one that does not (a
/// debug or plain build) could not read it, and keeps the login keychain.
/// Decided by the build, never by a setting. Elsewhere the system's.
fn for_a_new_device(macos: bool, with_profile: bool) -> Result<SecretBackend> {
    if macos && with_profile {
        Ok(SecretBackend::ProtectedKeychain)
    } else {
        device::system_keyring_backend().context("this system has no keyring OurStick can use yet")
    }
}

/// Whether this app's bundle carries the provisioning profile that allows
/// its keychain entitlements: `Contents/embedded.provisionprofile`.
fn carries_its_profile() -> bool {
    std::env::current_exe().ok().is_some_and(|exe| {
        exe.parent()
            .and_then(std::path::Path::parent)
            .is_some_and(|contents| contents.join("embedded.provisionprofile").is_file())
    })
}

/// Records this device if it is not yet: its row holds where its keys are
/// (so every key goes to one store: on a new Mac, without it, a received
/// stick's link went to the data protection keychain and the keys after it
/// to the login keychain), the name senders see and the Local Network step.
/// Called before any key is written; a device that only receives would
/// otherwise never get one. Blocking; takes the setup lock only when there
/// is no row. The caller then calls `local_network::remember`.
pub fn ensure_device(store: &dyn Store, data_dir: &std::path::Path) -> Result<()> {
    if store.device()?.is_some() {
        return Ok(());
    }
    let secrets = secrets(store)?;
    let backend = backend(store)?;
    record_device(store, data_dir, secrets.as_ref(), backend)
}

fn record_device(
    store: &dyn Store,
    data_dir: &std::path::Path,
    secrets: &dyn SecretStore,
    backend: SecretBackend,
) -> Result<()> {
    let _setup = setup_lock_waiting(data_dir)?;
    // Another program may have recorded it meanwhile: open_device checks.
    device::open_device(store, data_dir, secrets, backend)?;
    Ok(())
}

/// The setup lock, waited for up to three seconds: another command of this
/// app (or another program) may hold it a moment.
fn setup_lock_waiting(data_dir: &std::path::Path) -> Result<ourstick_core::locks::Lock> {
    let mut tries = 0;
    loop {
        match ourstick_core::locks::setup_lock(data_dir) {
            Err(ourstick_core::locks::LockError::SetupInUse) if tries < 30 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            lock => return Ok(lock?),
        }
    }
}

/// How many keys a move to the data protection keychain would read (macOS
/// asks once for each), if this device's keys are in the login keychain
/// and this build can keep them in the new one (ADR 0021, rule 3).
pub fn move_due(store: &dyn Store) -> Result<Option<usize>> {
    if for_a_new_device(cfg!(target_os = "macos"), carries_its_profile())?
        != SecretBackend::ProtectedKeychain
        || backend(store)? != SecretBackend::Keychain
    {
        return Ok(None);
    }
    let to_move = ourstick_core::secrets::secrets_to_move(store)?;
    Ok(Some(to_move.required.len() + to_move.optional.len()))
}

/// Moves the keys (the user said Continue): first the device's row, for one
/// that only received, then every key. Blocking; under the setup lock.
pub fn move_now(store: &dyn Store, data_dir: &std::path::Path) -> Result<usize> {
    let _setup = ourstick_core::locks::setup_lock(data_dir)?;
    let from = device::open_keyring(&SecretBackend::Keychain)?;
    if store.device()?.is_none() {
        device::open_device(store, data_dir, from.as_ref(), SecretBackend::Keychain)?;
    }
    let to = device::open_keyring(&SecretBackend::ProtectedKeychain)?;
    ourstick_core::check_secret_store(to.as_ref())?;
    Ok(ourstick_core::secrets::move_keys(
        store,
        from.as_ref(),
        to.as_ref(),
    )?)
}

/// At a start that began with the keys already moved: the login keychain's
/// old copies go, once every key reads from the new store. A failure only
/// waits for the next start.
pub fn finish_move(state: &crate::state::AppState) {
    let store = state.store.as_ref();
    if !state.started_moved || !store.old_keys_left().unwrap_or(false) {
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

/// The data protection keychain while the login keychain still holds the
/// old copies: a key deleted (a stick removed, a link changed, a received
/// stick forgotten) goes from both, so none is left behind there.
struct DeletedFromBoth {
    new: Box<dyn SecretStore>,
    old: Box<dyn SecretStore>,
}

impl SecretStore for DeletedFromBoth {
    fn describe(&self) -> String {
        self.new.describe()
    }

    fn durability(&self) -> Durability {
        self.new.durability()
    }

    fn put(&self, name: &SecretName, secret: &[u8]) -> Result<(), SecretError> {
        self.new.put(name, secret)
    }

    fn get(&self, name: &SecretName) -> Result<Option<Vec<u8>>, SecretError> {
        self.new.get(name)
    }

    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        self.new.delete(name)?;
        if let Err(error) = self.old.delete(name) {
            tracing::warn!(%error, "an old copy of a deleted key stays in the login keychain");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ourstick_core::secrets::MemorySecretStore;

    fn device(backend: SecretBackend) -> ourstick_core::store::Device {
        ourstick_core::store::Device {
            node_id: None,
            secret_backend: backend,
            created_at_ms: 0,
        }
    }

    #[test]
    fn a_new_mac_device_keeps_its_keys_in_the_data_protection_keychain_if_it_can() {
        assert_eq!(
            for_a_new_device(true, true).unwrap(),
            SecretBackend::ProtectedKeychain
        );
        assert_ne!(
            for_a_new_device(true, false).unwrap_or(SecretBackend::Keychain),
            SecretBackend::ProtectedKeychain,
            "without its profile it could not read it"
        );
    }

    #[test]
    fn a_device_keeps_the_backend_its_row_names() {
        let store = ourstick_core::MemoryStore::new();
        store
            .create_device(&device(SecretBackend::Keychain))
            .unwrap();
        assert_eq!(backend(&store).unwrap(), SecretBackend::Keychain);
    }

    #[test]
    fn a_device_that_only_received_keeps_the_systems_store() {
        let store = ourstick_core::MemoryStore::new();
        let stick = ourstick_core::StickSigningKey::from_bytes(&[2; 32]).stick_id();
        store
            .add_joined_stick(&ourstick_core::store::JoinedStick {
                kind: StickKind::App,
                ..ourstick_core::store::JoinedStick::new(stick, "/r", 0)
            })
            .unwrap();
        assert_eq!(
            backend(&store).unwrap(),
            device::system_keyring_backend().unwrap()
        );
    }

    #[test]
    fn nothing_to_move_without_a_profile_or_already_moved() {
        let store = ourstick_core::MemoryStore::new();
        assert_eq!(move_due(&store).unwrap(), None, "no device");
        store
            .create_device(&device(SecretBackend::ProtectedKeychain))
            .unwrap();
        assert_eq!(move_due(&store).unwrap(), None);
    }

    #[test]
    fn a_device_without_its_row_gets_one_once() {
        let store = ourstick_core::MemoryStore::new();
        let dir = crate::testing::TempFolder::new();
        let secrets = MemorySecretStore::new();
        record_device(&store, dir.path(), &secrets, SecretBackend::SecretService).unwrap();
        let made = store.device().unwrap().unwrap();
        assert_eq!(made.secret_backend, SecretBackend::SecretService);
        assert_eq!(made.node_id, None, "no node key of its own (ADR 0009)");
        // A second time keeps it as it is, and so does ensure_device,
        // without opening any keyring.
        record_device(&store, dir.path(), &secrets, SecretBackend::SecretService).unwrap();
        ensure_device(&store, dir.path()).unwrap();
        assert_eq!(
            store.device().unwrap().unwrap().created_at_ms,
            made.created_at_ms
        );
        store.set_receiver_name(Some("Μαρία")).unwrap();
        assert_eq!(store.receiver_name().unwrap().as_deref(), Some("Μαρία"));
    }

    #[test]
    fn the_setup_lock_is_waited_for_then_refused() {
        let dir = crate::testing::TempFolder::new();
        let held = ourstick_core::locks::setup_lock(dir.path()).unwrap();
        assert!(setup_lock_waiting(dir.path()).is_err());
        drop(held);
        assert!(setup_lock_waiting(dir.path()).is_ok());
    }

    #[test]
    fn a_key_deleted_goes_from_both_stores() {
        let new = MemorySecretStore::new();
        let old = MemorySecretStore::new();
        new.put(&SecretName::NodeKey, &[1; 32]).unwrap();
        old.put(&SecretName::NodeKey, &[1; 32]).unwrap();
        let both = DeletedFromBoth {
            new: Box::new(new),
            old: Box::new(old),
        };
        assert_eq!(both.get(&SecretName::NodeKey).unwrap(), Some(vec![1; 32]));
        both.delete(&SecretName::NodeKey).unwrap();
        assert_eq!(both.get(&SecretName::NodeKey).unwrap(), None);
        assert_eq!(both.old.get(&SecretName::NodeKey).unwrap(), None);
    }
}
