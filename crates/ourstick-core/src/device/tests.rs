use super::*;
use crate::secrets::MemorySecretStore;

fn environment<'a>(values: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
    move |name| {
        values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.into())
    }
}

#[test]
fn the_data_folder_follows_the_system() {
    let unix = [("HOME", "/home/alice"), ("XDG_DATA_HOME", "")];
    assert_eq!(
        data_dir_from(environment(&unix), false).unwrap(),
        PathBuf::from("/home/alice/.local/share/ourstick")
    );
    let xdg = [("HOME", "/home/alice"), ("XDG_DATA_HOME", "/data")];
    assert_eq!(
        data_dir_from(environment(&xdg), false).unwrap(),
        PathBuf::from("/data/ourstick")
    );
    let windows = [("LOCALAPPDATA", r"C:\Users\alice\AppData\Local")];
    assert_eq!(
        data_dir_from(environment(&windows), true).unwrap(),
        PathBuf::from(r"C:\Users\alice\AppData\Local").join("OurStick")
    );
    // $HOME alone does not count on Windows.
    assert!(matches!(
        data_dir_from(environment(&[("HOME", "/home/alice")]), true),
        Err(DeviceError::NoHome("%LOCALAPPDATA%"))
    ));
    let chosen = [("OURSTICK_DATA_DIR", "/elsewhere"), ("HOME", "/home/alice")];
    for windows in [false, true] {
        assert_eq!(
            data_dir_from(environment(&chosen), windows).unwrap(),
            PathBuf::from("/elsewhere")
        );
    }
}
use crate::store::MemoryStore;

#[test]
fn a_new_device_makes_no_node_key_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    let secrets = MemorySecretStore::new();
    open_device(&store, dir.path(), &secrets, SecretBackend::SecretService).unwrap();
    open_device(&store, dir.path(), &secrets, SecretBackend::SecretService).unwrap();
    assert_eq!(store.device().unwrap().unwrap().node_id, None);
    assert_eq!(secrets.get(&SecretName::NodeKey).unwrap(), None);
}

#[test]
fn keys_go_only_where_the_device_records_them() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    let secrets = MemorySecretStore::new();
    let key_file = SecretBackend::KeyFile("/keys/ourstick.keys".into());
    open_device(&store, dir.path(), &secrets, key_file.clone()).unwrap();
    open_device(&store, dir.path(), &secrets, key_file).unwrap();
    // E.g. the app, with the system keyring, on a device set up with a key
    // file by the CLI.
    match open_device(&store, dir.path(), &secrets, SecretBackend::SecretService) {
        Err(DeviceError::OtherSecretStore { recorded, given }) => {
            assert_eq!(recorded, "the key file /keys/ourstick.keys");
            assert_eq!(given, "the Secret Service");
        }
        other => panic!("{other:?}"),
    }
    let other_file = SecretBackend::KeyFile("/elsewhere.keys".into());
    assert!(matches!(
        open_device(&store, dir.path(), &secrets, other_file),
        Err(DeviceError::OtherSecretStore { .. })
    ));
}

#[test]
fn an_old_node_key_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    let secrets = MemorySecretStore::new();
    secrets.put(&SecretName::NodeKey, &[7; 32]).unwrap();
    let result = open_device(&store, dir.path(), &secrets, SecretBackend::SecretService);
    assert!(matches!(result, Err(DeviceError::NodeKeyExists { .. })));
    assert_eq!(
        secrets.get(&SecretName::NodeKey).unwrap(),
        Some(vec![7; 32].into())
    );
}

#[test]
fn a_new_stick_has_its_own_node_key_and_loads_with_the_same_keys() {
    let store = MemoryStore::new();
    let secrets = MemorySecretStore::new();
    let made = create_stick(
        &store,
        &secrets,
        Path::new("/tmp/s"),
        "s",
        StickKind::App,
        None,
    )
    .unwrap();
    let other = create_stick(
        &store,
        &secrets,
        Path::new("/tmp/t"),
        "t",
        StickKind::Cli,
        None,
    )
    .unwrap();
    assert_ne!(made.node_key, other.node_key);
    assert_eq!(made.stick.kind, StickKind::App);
    assert_eq!(other.stick.kind, StickKind::Cli);
    let stick = store.owned_stick(&made.stick.stick_id).unwrap().unwrap();
    assert_eq!(stick.node_id, Some(IrohConfig::node_id_of(&made.node_key)));
    let loaded = load_stick(&store, &secrets, stick.clone()).unwrap();
    assert_eq!(loaded.signing_key.stick_id(), made.stick.stick_id);
    assert_eq!(loaded.stick_key.as_bytes(), made.stick_key.as_bytes());
    assert_eq!(loaded.node_key, made.node_key);

    // A missing key is an error, never made again (rule 9).
    secrets
        .delete(&SecretName::StickNodeKey(stick.stick_id))
        .unwrap();
    assert!(matches!(
        load_stick(&store, &secrets, stick.clone()),
        Err(DeviceError::KeyMissing { .. })
    ));
    // A key that names another node is refused.
    secrets
        .put(&SecretName::StickNodeKey(stick.stick_id), &[1; 32])
        .unwrap();
    assert!(matches!(
        load_stick(&store, &secrets, stick.clone()),
        Err(DeviceError::NodeKeyMismatch { .. })
    ));
    // A signing key that is not the stick's is refused too.
    secrets
        .put(&SecretName::StickSigningKey(stick.stick_id), &[2; 32])
        .unwrap();
    assert!(matches!(
        load_stick(&store, &secrets, stick),
        Err(DeviceError::SigningKeyMismatch { .. })
    ));
}

#[test]
fn an_older_stick_uses_the_device_node_key() {
    let store = MemoryStore::new();
    let secrets = MemorySecretStore::new();
    let made = create_stick(
        &store,
        &secrets,
        Path::new("/tmp/s"),
        "s",
        StickKind::Cli,
        None,
    )
    .unwrap();
    // As a stick made before ADR 0009 is: no node key of its own.
    let older = OwnedStick {
        node_id: None,
        ..OwnedStick::new(
            StickSigningKey::from_bytes(&[3; 32]).stick_id(),
            "older",
            "/tmp/o",
            CIPHER,
            1,
        )
    };
    let older_keys = [
        (SecretName::StickSigningKey(older.stick_id), [3; 32]),
        (SecretName::StickKey(older.stick_id), [4; 32]),
    ];
    for (name, key) in &older_keys {
        secrets.put(name, key).unwrap();
    }
    store.add_owned_stick(&older).unwrap();
    // No device node key recorded: inconsistent, not made up.
    store
        .create_device(&Device {
            node_id: None,
            secret_backend: SecretBackend::SecretService,
            created_at_ms: 1,
        })
        .unwrap();
    assert!(matches!(
        load_stick(&store, &secrets, older.clone()),
        Err(DeviceError::NoDeviceNodeKey { .. })
    ));
    let with_device = MemoryStore::new();
    with_device
        .create_device(&Device {
            node_id: Some(IrohConfig::node_id_of(&[8; 32])),
            secret_backend: SecretBackend::SecretService,
            created_at_ms: 1,
        })
        .unwrap();
    with_device.add_owned_stick(&older).unwrap();
    secrets.put(&SecretName::NodeKey, &[8; 32]).unwrap();
    let loaded = load_stick(&with_device, &secrets, older.clone()).unwrap();
    assert_eq!(*loaded.node_key, [8; 32]);
    assert_ne!(loaded.node_key, made.node_key);
    // A device key that is not the one recorded: refused, as for a stick's.
    secrets.put(&SecretName::NodeKey, &[9; 32]).unwrap();
    assert!(matches!(
        load_stick(&with_device, &secrets, older),
        Err(DeviceError::NodeKeyMismatch { .. })
    ));
}

#[test]
fn two_sticks_never_share_a_node_key() {
    let store = MemoryStore::new();
    let secrets = MemorySecretStore::new();
    let first = create_stick(
        &store,
        &secrets,
        Path::new("/tmp/s"),
        "s",
        StickKind::Cli,
        None,
    )
    .unwrap();
    let mut clash = first.stick.clone();
    clash.stick_id = StickSigningKey::generate().stick_id();
    // The same node id for another stick: refused.
    store.add_owned_stick(&clash).unwrap_err();
    assert_eq!(store.owned_sticks().unwrap().len(), 1);
    // A new stick gets a node key of its own.
    let next = create_stick(
        &store,
        &secrets,
        Path::new("/tmp/s"),
        "s",
        StickKind::Cli,
        None,
    )
    .unwrap();
    assert_ne!(next.stick.stick_id, first.stick.stick_id);
    assert_ne!(next.node_key, first.node_key);
}

#[test]
fn a_store_this_system_does_not_have_is_refused_not_replaced() {
    // A key file needs its passphrase; no other system's store opens here.
    let path = std::path::PathBuf::from("/keys");
    assert!(open_keyring(&SecretBackend::KeyFile(path)).is_err());
    #[cfg(not(target_os = "macos"))]
    assert!(open_keyring(&SecretBackend::ProtectedKeychain).is_err());
    #[cfg(not(windows))]
    assert!(open_keyring(&SecretBackend::CredentialManager).is_err());
    #[cfg(not(target_os = "android"))]
    assert!(open_keyring(&SecretBackend::AndroidKeystore).is_err());
}

#[test]
fn a_new_stick_to_send_has_an_empty_folder_named_by_its_id_and_no_drive() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryStore::new();
    let secrets = MemorySecretStore::new();
    let made = create_sending_stick(&store, &secrets, dir.path(), "Holiday 2025").unwrap();
    let hex = data_encoding::HEXLOWER.encode(&made.stick.stick_id.to_bytes());
    // By its id, so a rename never moves it.
    assert_eq!(made.stick.root_path, dir.path().join("sent").join(hex));
    assert_eq!(
        std::fs::read_dir(&made.stick.root_path).unwrap().count(),
        0,
        "nothing writes to it"
    );
    assert_eq!(made.stick.name, "Holiday 2025");
    assert_eq!(made.stick.kind, StickKind::App);
    assert!(!made.stick.has_own_folder);
    assert!(!made.stick.is_drive());
    assert_eq!(
        store.owned_stick(&made.stick.stick_id).unwrap(),
        Some(made.stick.clone())
    );
    let loaded = load_stick(&store, &secrets, made.stick.clone()).unwrap();
    assert_eq!(loaded.node_key, made.node_key);
}

#[test]
fn only_an_app_stick_with_a_folder_of_its_own_is_a_drive() {
    let store = MemoryStore::new();
    let secrets = MemorySecretStore::new();
    let app = create_stick(
        &store,
        &secrets,
        Path::new("/tmp/s"),
        "s",
        StickKind::App,
        None,
    )
    .unwrap();
    let cli = create_stick(
        &store,
        &secrets,
        Path::new("/tmp/t"),
        "t",
        StickKind::Cli,
        None,
    )
    .unwrap();
    assert!(app.stick.is_drive());
    assert!(!cli.stick.is_drive());
}

#[cfg(unix)]
#[test]
fn only_a_data_folder_made_here_is_made_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let made = dir.path().join("made");
    create_private_dir(&made).unwrap();
    assert_eq!(mode(&made), 0o700);
    // A folder named by OURSTICK_DATA_DIR that was there keeps its mode.
    let there = dir.path().join("there");
    std::fs::create_dir(&there).unwrap();
    std::fs::set_permissions(&there, std::fs::Permissions::from_mode(0o755)).unwrap();
    create_private_dir(&there).unwrap();
    assert_eq!(mode(&there), 0o755);
}
