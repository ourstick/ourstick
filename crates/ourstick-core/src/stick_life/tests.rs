use std::cell::Cell;
use std::path::PathBuf;

use super::*;
use crate::device::{create_stick, load_stick};
use crate::manifest::DRIVE_MARKER;
use crate::secrets::MemorySecretStore;
use crate::store::{Device, LinkedEntry, MemoryStore, SecretBackend};
use crate::ticket::NodeId;

struct World {
    _dir: tempfile::TempDir,
    data: PathBuf,
    store: MemoryStore,
    secrets: MemorySecretStore,
    /// A folder linked to the app stick: never touched.
    linked: PathBuf,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let linked = dir.path().join("elsewhere/Films");
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(linked.join("film.mkv"), b"film").unwrap();
    World {
        data,
        linked,
        store: MemoryStore::new(),
        secrets: MemorySecretStore::new(),
        _dir: dir,
    }
}

/// An app stick with copied files, a linked folder and a cache folder.
fn app_stick(world: &World, name: &str) -> OwnedStick {
    let root = world.data.parent().unwrap().join("OurStick").join(name);
    std::fs::create_dir_all(root.join("photos")).unwrap();
    std::fs::write(root.join("photos/a.jpg"), b"a").unwrap();
    let keys = create_stick(&world.store, &world.secrets, &root, name, StickKind::App).unwrap();
    mark_drive(&root, &keys.stick.stick_id).unwrap();
    world
        .store
        .add_linked_entry(
            &keys.stick.stick_id,
            &LinkedEntry {
                name: "Films".to_owned(),
                path: world.linked.clone(),
                is_dir: true,
                disk: None,
                added_at_ms: 1,
            },
        )
        .unwrap();
    std::fs::create_dir_all(cache_dir(&world.data, &keys.stick.stick_id)).unwrap();
    keys.stick
}

fn has_secret(world: &World, name: SecretName) -> bool {
    world.secrets.get(&name).unwrap().is_some()
}

#[test]
fn a_new_link_keeps_the_files_and_drops_only_the_old_keys() {
    let world = world();
    let old = app_stick(&world, "For Maria");
    let new = give_new_link(&world.store, &world.secrets, &world.data, &old.stick_id).unwrap();
    let new_id = new.stick.stick_id;
    assert_ne!(new_id, old.stick_id);
    assert_ne!(new.stick.node_id, old.node_id);
    assert_eq!(new.stick.root_path, old.root_path);
    assert!(new.stick.root_path.join("photos/a.jpg").exists());
    assert!(drive_present(&new.stick.root_path, &new_id));
    assert_eq!(world.store.linked_entries(&new_id).unwrap().len(), 1);
    assert!(world.linked.join("film.mkv").exists());
    for name in [
        SecretName::StickSigningKey(old.stick_id),
        SecretName::StickKey(old.stick_id),
        SecretName::StickNodeKey(old.stick_id),
    ] {
        assert!(!has_secret(&world, name));
    }
    assert!(!cache_dir(&world.data, &old.stick_id).exists());
    assert!(world.store.retired_sticks().unwrap().is_empty());
    // The new keys load as the stick's.
    let loaded = load_stick(&world.store, &world.secrets, new.stick).unwrap();
    assert_eq!(loaded.node_key, new.node_key);
}

#[test]
fn a_new_link_stopped_halfway_is_finished_at_start() {
    let world = world();
    let old = app_stick(&world, "For Maria");
    // Stopped right after the transaction: the marker names the old id.
    let new = OwnedStick {
        stick_id: StickSigningKey::generate().stick_id(),
        node_id: Some(NodeId([9; 32])),
        ..old.clone()
    };
    world
        .store
        .replace_owned_stick(&old.stick_id, &new, 1)
        .unwrap();
    assert!(drive_present(&old.root_path, &old.stick_id));
    finish_new_links(&world.store, &world.secrets, &world.data).unwrap();
    assert!(drive_present(&new.root_path, &new.stick_id));
    assert!(new.root_path.join("photos/a.jpg").exists());
    assert!(!has_secret(&world, SecretName::StickKey(old.stick_id)));
    assert!(world.store.retired_sticks().unwrap().is_empty());
}

#[test]
fn removing_an_app_stick_deletes_its_copies_only() {
    let world = world();
    let going = app_stick(&world, "For Maria");
    let kept = app_stick(&world, "Family");
    remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &going.stick_id,
        &NoDrive,
    )
    .unwrap();
    assert!(!going.root_path.exists());
    assert!(
        world.linked.join("film.mkv").exists(),
        "a linked folder is never touched"
    );
    assert!(kept.root_path.join("photos/a.jpg").exists());
    assert!(!has_secret(&world, SecretName::StickKey(going.stick_id)));
    assert!(has_secret(&world, SecretName::StickKey(kept.stick_id)));
    assert!(!cache_dir(&world.data, &going.stick_id).exists());
    assert_eq!(world.store.owned_sticks().unwrap(), [kept]);
}

#[test]
fn a_command_line_sticks_folder_is_the_users_and_stays() {
    let world = world();
    let folder = world.data.parent().unwrap().join("shared");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("mine.txt"), b"mine").unwrap();
    let keys = create_stick(
        &world.store,
        &world.secrets,
        &folder,
        "shared",
        StickKind::Cli,
    )
    .unwrap();
    remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &keys.stick.stick_id,
        &NoDrive,
    )
    .unwrap();
    assert!(folder.join("mine.txt").exists());
    assert!(world.store.owned_sticks().unwrap().is_empty());
}

#[test]
fn an_older_stick_goes_without_the_devices_node_key() {
    let world = world();
    world
        .store
        .create_device(&Device {
            node_id: Some(IrohConfig::node_id_of(&[7; 32])),
            secret_backend: SecretBackend::SecretService,
            created_at_ms: 1,
        })
        .unwrap();
    world.secrets.put(&SecretName::NodeKey, &[7; 32]).unwrap();
    let mut older = Vec::new();
    for seed in [1u8, 2] {
        let stick = OwnedStick::new(
            StickSigningKey::from_bytes(&[seed; 32]).stick_id(),
            format!("older {seed}"),
            world.data.parent().unwrap().join(format!("older{seed}")),
            crate::cipher::CipherId::XChaCha20Poly1305,
            1,
        );
        world
            .secrets
            .put(&SecretName::StickSigningKey(stick.stick_id), &[seed; 32])
            .unwrap();
        world
            .secrets
            .put(&SecretName::StickKey(stick.stick_id), &[seed; 32])
            .unwrap();
        world.store.add_owned_stick(&stick).unwrap();
        older.push(stick);
    }
    // One gets a new link, the other is removed: the device key stays, and
    // nothing else of theirs is left.
    give_new_link(
        &world.store,
        &world.secrets,
        &world.data,
        &older[0].stick_id,
    )
    .unwrap();
    remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &older[1].stick_id,
        &NoDrive,
    )
    .unwrap();
    assert!(has_secret(&world, SecretName::NodeKey));
    for stick in &older {
        assert!(!has_secret(&world, SecretName::StickKey(stick.stick_id)));
    }
}

/// Fails to take the drive away the first `failures` times.
struct FlakyDrive {
    failures: Cell<u32>,
}

impl DriveOps for FlakyDrive {
    fn take_away(&self, _stick: &OwnedStick) -> Result<(), String> {
        if self.failures.get() > 0 {
            self.failures.set(self.failures.get() - 1);
            return Err("a file on the stick is open".to_owned());
        }
        Ok(())
    }
}

#[test]
fn a_removal_stopped_halfway_is_finished_later_and_twice_is_fine() {
    let world = world();
    let going = app_stick(&world, "For Maria");
    let drive = FlakyDrive {
        failures: Cell::new(1),
    };
    let stopped = remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &going.stick_id,
        &drive,
    );
    assert!(matches!(stopped, Err(LifeError::Drive(_))), "{stopped:?}");
    // Never shared again meanwhile, and its keys are still there.
    assert!(world.store.owned_sticks().unwrap().is_empty());
    assert!(has_secret(&world, SecretName::StickKey(going.stick_id)));
    for _ in 0..2 {
        let failed = finish_removals(&world.store, &world.secrets, &world.data, &drive).unwrap();
        assert!(failed.is_empty());
    }
    assert!(world.store.removing_sticks().unwrap().is_empty());
    assert!(!has_secret(&world, SecretName::StickKey(going.stick_id)));
}

/// A secret store whose `put` fails from the `fail_from`th call on, and
/// that remembers what it was asked to store.
struct FailingPuts {
    inner: MemorySecretStore,
    puts: std::sync::atomic::AtomicU32,
    fail_from: u32,
    asked: std::sync::Mutex<Vec<SecretName>>,
}

impl SecretStore for FailingPuts {
    fn describe(&self) -> String {
        "failing".to_owned()
    }
    fn durability(&self) -> crate::secrets::Durability {
        self.inner.durability()
    }
    fn put(&self, name: &SecretName, secret: &[u8]) -> Result<(), SecretError> {
        let puts = self.puts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.asked.lock().unwrap().push(name.clone());
        if puts >= self.fail_from {
            return Err(SecretError::Unavailable {
                store: "failing".to_owned(),
                reason: "test".to_owned(),
            });
        }
        self.inner.put(name, secret)
    }
    fn get(&self, name: &SecretName) -> Result<Option<Vec<u8>>, SecretError> {
        self.inner.get(name)
    }
    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        self.inner.delete(name)
    }
}

#[test]
fn a_new_link_that_fails_leaves_no_keys_behind() {
    let world = world();
    let stick = app_stick(&world, "For Maria");
    let failing = FailingPuts {
        inner: MemorySecretStore::new(),
        puts: std::sync::atomic::AtomicU32::new(0),
        fail_from: 3,
        asked: std::sync::Mutex::default(),
    };
    let result = give_new_link(&world.store, &failing, &world.data, &stick.stick_id);
    assert!(result.is_err());
    let asked = failing.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 3);
    for name in asked {
        assert_eq!(failing.inner.get(&name).unwrap(), None, "{name:?} was left");
    }
    // Nothing changed for the stick.
    assert_eq!(world.store.owned_sticks().unwrap(), [stick]);
}

#[cfg(unix)]
#[test]
fn a_new_link_whose_marker_cannot_be_fixed_waits_and_others_go_on() {
    use std::os::unix::fs::PermissionsExt;
    let world = world();
    let stuck = app_stick(&world, "Stuck");
    let fine = app_stick(&world, "Fine");
    let mut news = Vec::new();
    for old in [&stuck, &fine] {
        let new = OwnedStick {
            stick_id: StickSigningKey::generate().stick_id(),
            node_id: Some(NodeId([news.len() as u8 + 20; 32])),
            ..old.clone()
        };
        world
            .store
            .replace_owned_stick(&old.stick_id, &new, 1)
            .unwrap();
        news.push(new);
    }
    // The stuck stick's marker cannot be written (a read-only drive).
    let marker = stuck.root_path.join(DRIVE_MARKER);
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o400)).unwrap();
    let failed = finish_new_links(&world.store, &world.secrets, &world.data).unwrap();
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).unwrap();
    if failed.is_empty() {
        return; // Running as root: nothing is read-only.
    }
    assert_eq!(failed[0].0, "Stuck");
    assert!(drive_present(&news[1].root_path, &news[1].stick_id));
    // The stuck one's old id waits for the next start; it then goes.
    assert_eq!(world.store.retired_sticks().unwrap(), [stuck.stick_id]);
    assert!(finish_new_links(&world.store, &world.secrets, &world.data)
        .unwrap()
        .is_empty());
    assert!(drive_present(&news[0].root_path, &news[0].stick_id));
    assert!(world.store.retired_sticks().unwrap().is_empty());
}
