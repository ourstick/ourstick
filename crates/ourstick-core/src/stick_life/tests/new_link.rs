//! "Give it a new link" and finishing it at start.

use super::*;

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
    fn get(&self, name: &SecretName) -> Result<Option<crate::secrets::Secret>, SecretError> {
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
    let marker = stuck.root_path.join(crate::manifest::DRIVE_MARKER);
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o400)).unwrap();
    let failed = finish_new_links(&world.store, &world.secrets, &world.data).unwrap();
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).unwrap();
    if failed.is_empty() {
        return; // Running as root: nothing is read-only.
    }
    assert_eq!(failed[0].0, "Stuck");
    assert!(drive_present(&news[1].root_path, &news[1].stick_id));
    // The stuck one's old id waits for the next start; it then goes.
    let waiting: Vec<StickId> = world
        .store
        .retired_sticks()
        .unwrap()
        .iter()
        .map(|entry| entry.old)
        .collect();
    assert_eq!(waiting, [stuck.stick_id]);
    assert!(finish_new_links(&world.store, &world.secrets, &world.data)
        .unwrap()
        .is_empty());
    assert!(drive_present(&news[0].root_path, &news[0].stick_id));
    assert!(world.store.retired_sticks().unwrap().is_empty());
}

/// A new link stopped right after its transaction: the marker names `old`.
fn stopped_new_link(world: &World, old: &OwnedStick, seed: u8) -> OwnedStick {
    let new = OwnedStick {
        stick_id: StickSigningKey::generate().stick_id(),
        node_id: Some(NodeId([seed; 32])),
        ..old.clone()
    };
    world
        .store
        .replace_owned_stick(&old.stick_id, &new, u64::from(seed))
        .unwrap();
    new
}

#[test]
fn a_new_link_waits_while_its_drive_is_away_and_finishes_when_back() {
    let world = world();
    let old = app_stick(&world, "For Maria");
    let new = stopped_new_link(&world, &old, 30);
    // Its drive is not there at start (unplugged; on macOS, not attached).
    let away = old.root_path.with_file_name("away");
    std::fs::rename(&old.root_path, &away).unwrap();
    let failed = finish_new_links(&world.store, &world.secrets, &world.data).unwrap();
    assert!(failed.is_empty());
    assert_eq!(world.store.retired_sticks().unwrap().len(), 1, "kept");
    assert!(has_secret(&world, SecretName::StickKey(old.stick_id)));
    // Back (mounted): the marker names the stick, and the old id goes.
    std::fs::rename(&away, &old.root_path).unwrap();
    finish_new_links(&world.store, &world.secrets, &world.data).unwrap();
    assert!(drive_present(&new.root_path, &new.stick_id));
    assert!(world.store.retired_sticks().unwrap().is_empty());
    assert!(!has_secret(&world, SecretName::StickKey(old.stick_id)));
}

#[test]
fn new_links_one_after_another_still_find_the_stick() {
    let world = world();
    let first = app_stick(&world, "For Maria");
    // A → B stopped before the marker; then B → C, also stopped.
    let second = stopped_new_link(&world, &first, 31);
    let third = stopped_new_link(&world, &second, 32);
    assert!(drive_present(&first.root_path, &first.stick_id));
    finish_new_links(&world.store, &world.secrets, &world.data).unwrap();
    assert!(drive_present(&third.root_path, &third.stick_id));
    assert!(world.store.retired_sticks().unwrap().is_empty());
}

#[test]
fn a_new_link_tidies_only_its_own_old_id() {
    let world = world();
    let away_stick = app_stick(&world, "Away");
    let other = app_stick(&world, "Other");
    stopped_new_link(&world, &away_stick, 33);
    // The first stick's drive is away when the other gets a new link.
    let away = away_stick.root_path.with_file_name("away-drive");
    std::fs::rename(&away_stick.root_path, &away).unwrap();
    give_new_link(&world.store, &world.secrets, &world.data, &other.stick_id).unwrap();
    let left: Vec<StickId> = world
        .store
        .retired_sticks()
        .unwrap()
        .iter()
        .map(|entry| entry.old)
        .collect();
    assert_eq!(left, [away_stick.stick_id]);
}

/// The old ids still waiting.
fn waiting(world: &World) -> Vec<StickId> {
    world
        .store
        .retired_sticks()
        .unwrap()
        .iter()
        .map(|entry| entry.old)
        .collect()
}

fn finish(world: &World) {
    let failed = finish_new_links(&world.store, &world.secrets, &world.data).unwrap();
    assert!(failed.is_empty(), "{failed:?}");
}

#[test]
fn a_chain_of_new_links_waits_for_its_drive_too() {
    let world = world();
    let first = app_stick(&world, "For Maria");
    let second = stopped_new_link(&world, &first, 40);
    let third = stopped_new_link(&world, &second, 41);
    let away = first.root_path.with_file_name("away");
    std::fs::rename(&first.root_path, &away).unwrap();
    finish(&world);
    assert_eq!(waiting(&world).len(), 2);
    std::fs::rename(&away, &first.root_path).unwrap();
    finish(&world);
    assert!(drive_present(&third.root_path, &third.stick_id));
    assert!(waiting(&world).is_empty());
}

#[test]
fn an_old_id_whose_stick_was_removed_since_goes_and_touches_nothing() {
    let world = world();
    let first = app_stick(&world, "For Maria");
    let second = stopped_new_link(&world, &first, 42);
    // Removed with its marker still naming the first id: nothing deleted.
    remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &second.stick_id,
        &NoDrive,
    )
    .unwrap();
    assert!(first.root_path.join("photos/a.jpg").exists());
    finish(&world);
    assert!(waiting(&world).is_empty());
    assert!(!has_secret(&world, SecretName::StickKey(first.stick_id)));
    assert!(
        drive_present(&first.root_path, &first.stick_id),
        "left as it was"
    );
}

#[test]
fn a_command_line_stick_has_no_marker_to_wait_for() {
    let world = world();
    let folder = world.data.parent().unwrap().join("shared");
    std::fs::create_dir_all(&folder).unwrap();
    let keys = create_stick(
        &world.store,
        &world.secrets,
        &folder,
        "shared",
        StickKind::Cli,
        None,
    )
    .unwrap();
    give_new_link(
        &world.store,
        &world.secrets,
        &world.data,
        &keys.stick.stick_id,
    )
    .unwrap();
    assert!(waiting(&world).is_empty());
    assert!(!has_secret(
        &world,
        SecretName::StickKey(keys.stick.stick_id)
    ));
}

#[test]
fn a_marker_naming_another_sticks_old_id_is_never_claimed() {
    let world = world();
    let mine = app_stick(&world, "Mine");
    let theirs = app_stick(&world, "Theirs");
    let theirs_new = stopped_new_link(&world, &theirs, 43);
    // Their drive is where mine should be: my folder's marker names their
    // old id.
    mark_drive(&mine.root_path, &theirs.stick_id).unwrap();
    finish(&world);
    assert!(
        drive_present(&mine.root_path, &theirs.stick_id),
        "not claimed"
    );
    assert!(drive_present(&theirs_new.root_path, &theirs_new.stick_id));
    assert!(waiting(&world).is_empty());
}

#[test]
fn a_marker_naming_some_other_id_is_left_and_the_old_id_waits() {
    let world = world();
    let old = app_stick(&world, "For Maria");
    let new = stopped_new_link(&world, &old, 44);
    let stranger = StickSigningKey::generate().stick_id();
    mark_drive(&new.root_path, &stranger).unwrap();
    finish(&world);
    assert!(drive_present(&new.root_path, &stranger));
    assert_eq!(waiting(&world), [old.stick_id]);
}

#[test]
fn a_new_link_given_while_its_drive_is_away_finishes_when_it_is_back() {
    let world = world();
    let old = app_stick(&world, "For Maria");
    let away = old.root_path.with_file_name("away");
    std::fs::rename(&old.root_path, &away).unwrap();
    let new = give_new_link(&world.store, &world.secrets, &world.data, &old.stick_id).unwrap();
    assert_eq!(waiting(&world), [old.stick_id]);
    assert!(has_secret(&world, SecretName::StickKey(old.stick_id)));
    std::fs::rename(&away, &old.root_path).unwrap();
    finish(&world);
    assert!(drive_present(&new.stick.root_path, &new.stick.stick_id));
    assert!(waiting(&world).is_empty());
}
