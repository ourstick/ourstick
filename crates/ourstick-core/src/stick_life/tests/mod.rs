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

/// An app stick made before ADR 0028: a folder of its own with files in
/// it, a linked folder and a cache folder.
fn app_stick(world: &World, name: &str) -> OwnedStick {
    let root = world.data.parent().unwrap().join("OurStick").join(name);
    std::fs::create_dir_all(root.join("photos")).unwrap();
    std::fs::write(root.join("photos/a.jpg"), b"a").unwrap();
    let keys = create_stick(
        &world.store,
        &world.secrets,
        &root,
        name,
        StickKind::App,
        None,
    )
    .unwrap();
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
                at_top: false,
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
fn removing_an_old_app_stick_keeps_its_files_and_drops_its_marker() {
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
    // ADR 0028 §7: a plain folder of the user's from now on.
    assert!(going.root_path.join("photos/a.jpg").exists());
    assert!(!going.root_path.join(DRIVE_MARKER).exists());
    assert!(
        world.linked.join("film.mkv").exists(),
        "a linked folder is never touched"
    );
    assert!(kept.root_path.join("photos/a.jpg").exists());
    assert!(crate::share::drive_marked_by(
        &kept.root_path,
        &kept.stick_id
    ));
    assert!(!has_secret(&world, SecretName::StickKey(going.stick_id)));
    assert!(has_secret(&world, SecretName::StickKey(kept.stick_id)));
    assert!(!cache_dir(&world.data, &going.stick_id).exists());
    assert_eq!(world.store.owned_sticks().unwrap(), [kept]);
}

#[test]
fn a_marker_naming_another_stick_is_left_when_one_is_removed() {
    let world = world();
    let going = app_stick(&world, "For Maria");
    let other = StickSigningKey::from_bytes(&[9; 32]).stick_id();
    mark_drive(&going.root_path, &other).unwrap();
    remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &going.stick_id,
        &NoDrive,
    )
    .unwrap();
    assert!(crate::share::drive_marked_by(&going.root_path, &other));
}

/// A drive that must never be asked for: a stick of ADR 0028 has none.
struct NoDriveAtAll;

impl DriveOps for NoDriveAtAll {
    fn take_away(&self, stick: &OwnedStick) -> Result<(), String> {
        panic!("{} has no drive to take away", stick.name)
    }
}

/// A stick of ADR 0028: an empty folder of its id, and a linked folder.
fn sending_stick(world: &World, name: &str) -> OwnedStick {
    let keys = crate::device::create_sending_stick(&world.store, &world.secrets, &world.data, name)
        .unwrap();
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
                at_top: false,
            },
        )
        .unwrap();
    keys.stick
}

#[test]
fn removing_a_stick_with_no_folder_of_its_own_deletes_only_its_empty_folder() {
    let world = world();
    let going = sending_stick(&world, "Holiday 2025");
    remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &going.stick_id,
        &NoDriveAtAll,
    )
    .unwrap();
    assert!(!going.root_path.exists());
    assert!(world.linked.join("film.mkv").exists());
    assert!(!has_secret(&world, SecretName::StickKey(going.stick_id)));
    assert!(world.store.owned_sticks().unwrap().is_empty());
}

#[test]
fn a_new_link_for_a_stick_with_no_folder_of_its_own_waits_for_no_marker() {
    let world = world();
    let old = sending_stick(&world, "Holiday 2025");
    let new = give_new_link(&world.store, &world.secrets, &world.data, &old.stick_id).unwrap();
    assert!(!new.stick.has_own_folder);
    assert_eq!(new.stick.root_path, old.root_path);
    assert!(!old.root_path.join(DRIVE_MARKER).exists());
    // The old id is gone at once: no drive to wait for.
    assert!(world.store.retired_sticks().unwrap().is_empty());
    assert!(!has_secret(&world, SecretName::StickKey(old.stick_id)));
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
        None,
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

/// Takes the drive away only while the stick's marker is still there.
struct MarkerDrive;

impl DriveOps for MarkerDrive {
    fn take_away(&self, stick: &OwnedStick) -> Result<(), String> {
        if crate::share::drive_marked_by(&stick.root_path, &stick.stick_id) {
            Ok(())
        } else {
            Err("the marker is gone: the letter can no longer be found".to_owned())
        }
    }
}

#[test]
fn the_drive_is_taken_away_before_the_stick_is_emptied() {
    let world = world();
    let going = app_stick(&world, "For Maria");
    remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &going.stick_id,
        &MarkerDrive,
    )
    .unwrap();
    assert!(world.store.removing_sticks().unwrap().is_empty());
}

#[test]
fn a_removal_running_now_is_left_to_its_own_command() {
    let world = world();
    let going = app_stick(&world, "For Maria");
    let drive = FlakyDrive {
        failures: Cell::new(1),
    };
    let _ = remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &going.stick_id,
        &drive,
    );
    let lock = crate::locks::stick_lock(&world.data, &going).unwrap();
    let failed = finish_removals(&world.store, &world.secrets, &world.data, &NoDrive).unwrap();
    assert!(failed.is_empty());
    assert_eq!(world.store.removing_sticks().unwrap().len(), 1);
    drop(lock);
    finish_removals(&world.store, &world.secrets, &world.data, &NoDrive).unwrap();
    assert!(world.store.removing_sticks().unwrap().is_empty());
}

mod new_link;
