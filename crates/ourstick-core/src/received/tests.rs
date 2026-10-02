use super::*;
use crate::cipher::CipherId;
use crate::record::ManifestRecord;
use crate::secrets::MemorySecretStore;
use crate::stick_id::StickSigningKey;
use crate::store::{JoinedStick, MemoryStore, OwnedStick};

fn stick_id(seed: u8) -> StickId {
    StickSigningKey::from_bytes(&[seed; 32]).stick_id()
}

fn owned(seed: u8, name: &str) -> OwnedStick {
    OwnedStick {
        node_id: Some(crate::ticket::NodeId([seed; 32])),
        ..OwnedStick::new(stick_id(seed), name, "/s", CipherId::XChaCha20Poly1305, 1)
    }
}

fn received(store: &MemoryStore, seed: u8, kind: StickKind, output: &Path) {
    let joined = JoinedStick {
        kind,
        name: Some("Holiday".to_owned()),
        ..JoinedStick::new(stick_id(seed), output, 1)
    };
    store.add_joined_stick(&joined).unwrap();
}

#[test]
fn a_link_to_one_of_this_devices_own_sticks_is_refused() {
    let store = MemoryStore::new();
    store.add_owned_stick(&owned(1, "Photos")).unwrap();
    assert!(matches!(
        check_link(&store, &stick_id(1), StickKind::App),
        Err(ReceivedError::OwnStick(name)) if name == "Photos"
    ));
    // A stick it gave a new sharing link: the old id is retired.
    store
        .replace_owned_stick(&stick_id(1), &owned(2, "Photos"), 5)
        .unwrap();
    assert!(matches!(
        check_link(&store, &stick_id(1), StickKind::Cli),
        Err(ReceivedError::RetiredStick)
    ));
    assert!(check_link(&store, &stick_id(3), StickKind::App).is_ok());
    // A stick being removed is still this device's own.
    store.add_owned_stick(&owned(7, "Films")).unwrap();
    store.mark_removing(&stick_id(7)).unwrap();
    assert!(matches!(
        check_link(&store, &stick_id(7), StickKind::Cli),
        Err(ReceivedError::OwnStick(name)) if name == "Films"
    ));
}

#[test]
fn an_unknown_stick_is_not_forgotten() {
    let (store, secrets, dir, _output) = world();
    let data = dir.path().join("data");
    let lock = download_lock(&data, &stick_id(8)).unwrap();
    assert!(matches!(
        forget_received(&store, &secrets, &data, &stick_id(8), &lock),
        Err(ReceivedError::Store(StoreError::StickNotFound))
    ));
}

#[test]
fn the_window_takes_over_a_stick_get_received_but_not_its_own_twice() {
    let store = MemoryStore::new();
    received(&store, 4, StickKind::Cli, Path::new("/d/one"));
    assert!(check_link(&store, &stick_id(4), StickKind::App).is_ok());
    assert!(check_link(&store, &stick_id(4), StickKind::Cli).is_ok());
    received(&store, 5, StickKind::App, Path::new("/d/two"));
    assert!(matches!(
        check_link(&store, &stick_id(5), StickKind::App),
        Err(ReceivedError::AlreadyReceived(name)) if name == "Holiday"
    ));
    // `get` is told at once, before it makes a folder or starts networking.
    assert!(matches!(
        check_link(&store, &stick_id(5), StickKind::Cli),
        Err(ReceivedError::ReceivedByApp(folder)) if folder == Path::new("/d/two")
    ));
}

#[test]
fn a_stick_without_a_name_yet_is_called_by_its_id() {
    assert_eq!(display_name(Some("Holiday"), &stick_id(1)), "Holiday");
    let unnamed = display_name(None, &stick_id(1));
    assert!(
        unnamed.starts_with("Stick ") && unnamed.len() == 14,
        "{unnamed}"
    );
    assert_eq!(display_name(Some(""), &stick_id(1)), unnamed);
}

/// A received stick with a ticket, waiting blobs, a received file and a
/// record floor.
fn world() -> (MemoryStore, MemorySecretStore, tempfile::TempDir, PathBuf) {
    let store = MemoryStore::new();
    let secrets = MemorySecretStore::new();
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("Holiday");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("beach.jpg"), b"sand").unwrap();
    crate::download::mark_received_folder(&output, &stick_id(6)).unwrap();
    received(&store, 6, StickKind::App, &output);
    secrets
        .put(&SecretName::Ticket(stick_id(6)), b"ticket")
        .unwrap();
    let data = dir.path().join("data");
    let blobs = received_store_dir(&data, &stick_id(6));
    std::fs::create_dir_all(&blobs).unwrap();
    std::fs::write(blobs.join("blob"), b"x").unwrap();
    store
        .accept_record(&ManifestRecord {
            stick_id: stick_id(6),
            seq: 7,
            manifest_hash: [1; 32],
        })
        .unwrap();
    (store, secrets, dir, output)
}

#[test]
fn forgetting_keeps_the_files_and_the_floor_and_drops_the_rest() {
    let (store, secrets, dir, output) = world();
    let data = dir.path().join("data");
    let lock = download_lock(&data, &stick_id(6)).unwrap();
    forget_received(&store, &secrets, &data, &stick_id(6), &lock).unwrap();
    assert!(store.joined_stick(&stick_id(6)).unwrap().is_none());
    assert!(store.forgetting_joined_sticks().unwrap().is_empty());
    assert!(secrets
        .get(&SecretName::Ticket(stick_id(6)))
        .unwrap()
        .is_none());
    assert!(!received_store_dir(&data, &stick_id(6)).exists());
    assert_eq!(std::fs::read(output.join("beach.jpg")).unwrap(), b"sand");
    // Its mark is gone with it: the folder is only the user's files.
    assert!(!output.join(crate::download::RECEIVED_MARKER).exists());
    assert_eq!(store.record_floor(&stick_id(6)).unwrap().unwrap().seq, 7);
    // Forgotten: added again from its link, it is a new received stick.
    assert!(check_link(&store, &stick_id(6), StickKind::App).is_ok());
}

#[test]
fn a_forget_stopped_halfway_is_hidden_and_finished_at_start() {
    let (store, secrets, dir, _output) = world();
    let data = dir.path().join("data");
    store
        .change_joined_stick(&stick_id(6), &JoinedChange::Forgetting)
        .unwrap();
    assert!(store.joined_sticks().unwrap().is_empty());
    // Being forgotten by its own command right now: left to it.
    let lock = download_lock(&data, &stick_id(6)).unwrap();
    assert!(finish_forgetting(&store, &secrets, &data)
        .unwrap()
        .is_empty());
    assert_eq!(store.forgetting_joined_sticks().unwrap().len(), 1);
    drop(lock);
    assert!(finish_forgetting(&store, &secrets, &data)
        .unwrap()
        .is_empty());
    assert!(store.forgetting_joined_sticks().unwrap().is_empty());
    assert!(!received_store_dir(&data, &stick_id(6)).exists());
    // And a forget that finds it marked finishes it too.
    let (store, secrets, dir, _output) = world();
    let data = dir.path().join("data");
    store
        .change_joined_stick(&stick_id(6), &JoinedChange::Forgetting)
        .unwrap();
    let lock = download_lock(&data, &stick_id(6)).unwrap();
    forget_received(&store, &secrets, &data, &stick_id(6), &lock).unwrap();
    assert!(store.forgetting_joined_sticks().unwrap().is_empty());
}

#[test]
fn every_program_finds_a_received_sticks_blobs_in_one_place() {
    let dir = received_store_dir(Path::new("/data"), &stick_id(1));
    assert!(dir.starts_with("/data/joined"));
    assert_eq!(dir.file_name().unwrap().len(), 64);
}

#[test]
fn only_its_own_mark_is_taken_out() {
    let dir = tempfile::tempdir().unwrap();
    crate::download::mark_received_folder(dir.path(), &stick_id(1)).unwrap();
    crate::download::unmark_received_folder(dir.path(), &stick_id(2));
    assert!(crate::download::received_folder_there(
        dir.path(),
        &stick_id(1)
    ));
    crate::download::unmark_received_folder(dir.path(), &stick_id(1));
    assert!(!dir.path().join(crate::download::RECEIVED_MARKER).exists());
    // A folder that is not there (an unplugged disk): nothing happens.
    crate::download::unmark_received_folder(&dir.path().join("gone"), &stick_id(1));
}
