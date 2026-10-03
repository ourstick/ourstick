use std::sync::Mutex;

use super::*;
use crate::stick_id::StickSigningKey;

/// Fast Argon2 parameters, so tests do not spend 64 MiB and seconds per file.
const FAST: KeyFileParams = KeyFileParams {
    memory_kib: 64,
    iterations: 1,
    parallelism: 1,
};

fn stick_id() -> StickId {
    StickSigningKey::from_bytes(&[1; 32]).stick_id()
}

/// A durable-claiming store whose behaviour can be broken on purpose, to test
/// that [`check_secret_store`] catches stores that lie.
#[derive(Default)]
struct FaultyStore {
    inner: MemorySecretStore,
    forget_writes: bool,
    corrupt_reads: bool,
    ignore_deletes: bool,
    fail_writes: bool,
    writes: Mutex<u32>,
}

impl SecretStore for FaultyStore {
    fn describe(&self) -> String {
        "faulty".to_owned()
    }
    fn durability(&self) -> Durability {
        Durability::Durable
    }
    fn put(&self, name: &SecretName, secret: &[u8]) -> Result<(), SecretError> {
        *self.writes.lock().unwrap() += 1;
        if self.fail_writes {
            return Err(SecretError::Backend {
                store: self.describe(),
                reason: "disk full".to_owned(),
            });
        }
        if self.forget_writes {
            return Ok(());
        }
        self.inner.put(name, secret)
    }
    fn get(&self, name: &SecretName) -> Result<Option<Vec<u8>>, SecretError> {
        let value = self.inner.get(name)?;
        Ok(if self.corrupt_reads {
            value.map(|mut bytes| {
                bytes[0] ^= 1;
                bytes
            })
        } else {
            value
        })
    }
    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        if self.ignore_deletes {
            return Ok(());
        }
        self.inner.delete(name)
    }
}

#[test]
fn secret_names_are_stable_and_distinct() {
    let hex = data_encoding::HEXLOWER.encode(&stick_id().to_bytes());
    assert_eq!(SecretName::NodeKey.as_key(), "node-key");
    assert_eq!(
        SecretName::StickSigningKey(stick_id()).as_key(),
        format!("stick/{hex}/signing-key")
    );
    assert_eq!(
        SecretName::StickKey(stick_id()).as_key(),
        format!("stick/{hex}/stick-key")
    );
    assert_eq!(
        SecretName::StickNodeKey(stick_id()).as_key(),
        format!("stick/{hex}/node-key")
    );
    assert_eq!(
        SecretName::Ticket(stick_id()).as_key(),
        format!("stick/{hex}/ticket")
    );
    assert_eq!(
        SecretName::ReaderKey(stick_id()).as_key(),
        format!("stick/{hex}/reader-key")
    );
    assert_eq!(
        SecretName::HealthCheck([0xAB; 8]).as_key(),
        "health-check/abababababababab"
    );
    assert_eq!(format!("{:?}", SecretName::NodeKey), "SecretName(node-key)");
}

#[test]
fn memory_store_roundtrips_and_is_not_durable() {
    let store = MemorySecretStore::new();
    let name = SecretName::StickKey(stick_id());
    assert_eq!(store.get(&name).unwrap(), None);
    store.put(&name, b"one").unwrap();
    store.put(&name, b"two").unwrap();
    assert_eq!(store.get(&name).unwrap(), Some(b"two".to_vec()));
    store.delete(&name).unwrap();
    store.delete(&name).unwrap();
    assert_eq!(store.get(&name).unwrap(), None);
    assert_eq!(store.durability(), Durability::Ephemeral);
    assert_eq!(store.describe(), "memory (for tests)");
}

#[test]
fn startup_check_rejects_memory_store() {
    assert!(matches!(
        check_secret_store(&MemorySecretStore::new()),
        Err(SecretError::NotDurable { .. })
    ));
}

#[test]
fn startup_check_accepts_a_store_that_works() {
    let store = FaultyStore::default();
    check_secret_store(&store).unwrap();
    // It really wrote something, and left nothing behind.
    assert_eq!(*store.writes.lock().unwrap(), 1);
}

#[test]
fn startup_check_catches_stores_that_lie() {
    let cases = [
        FaultyStore {
            forget_writes: true,
            ..Default::default()
        },
        FaultyStore {
            corrupt_reads: true,
            ..Default::default()
        },
        FaultyStore {
            ignore_deletes: true,
            ..Default::default()
        },
    ];
    for store in cases {
        assert!(matches!(
            check_secret_store(&store),
            Err(SecretError::Unreliable { .. })
        ));
    }
}

#[test]
fn startup_check_reports_write_errors() {
    let store = FaultyStore {
        fail_writes: true,
        ..Default::default()
    };
    assert!(matches!(
        check_secret_store(&store),
        Err(SecretError::Backend { .. })
    ));
}

#[test]
fn key_file_roundtrips_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys.oskf");
    let name = SecretName::StickSigningKey(stick_id());

    let store = KeyFileStore::create(&path, "correct horse", FAST).unwrap();
    check_secret_store(&store).unwrap();
    store.put(&name, &[7; 32]).unwrap();
    store.put(&SecretName::NodeKey, b"node").unwrap();
    store.delete(&SecretName::NodeKey).unwrap();
    drop(store);

    let reopened = KeyFileStore::open(&path, "correct horse").unwrap();
    assert_eq!(reopened.get(&name).unwrap(), Some(vec![7; 32]));
    assert_eq!(reopened.get(&SecretName::NodeKey).unwrap(), None);
    assert_eq!(reopened.path(), path);
    assert_eq!(reopened.durability(), Durability::Durable);
    assert!(reopened.describe().contains("keys.oskf"));
}

#[test]
fn key_file_does_not_contain_secrets_in_clear() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys.oskf");
    let store = KeyFileStore::create(&path, "correct horse", FAST).unwrap();
    let secret = b"a very recognisable secret value";
    store
        .put(&SecretName::StickKey(stick_id()), secret)
        .unwrap();

    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.windows(secret.len()).any(|window| window == secret));
    assert!(!bytes.windows(9).any(|window| window == b"stick-key"));
}

#[test]
fn key_file_rejects_wrong_passphrase_and_damage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys.oskf");
    KeyFileStore::create(&path, "correct horse", FAST).unwrap();
    assert!(matches!(
        KeyFileStore::open(&path, "wrong horse"),
        Err(SecretError::WrongPassphraseOrDamaged)
    ));

    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        KeyFileStore::open(&path, "correct horse"),
        Err(SecretError::WrongPassphraseOrDamaged)
    ));
}

#[test]
fn key_file_header_is_authenticated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys.oskf");
    KeyFileStore::create(&path, "correct horse", FAST).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    // Flip a bit of the salt: the key changes, so decryption must fail.
    bytes[20] ^= 1;
    std::fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        KeyFileStore::open(&path, "correct horse"),
        Err(SecretError::WrongPassphraseOrDamaged)
    ));
}

#[test]
fn key_file_create_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys.oskf");
    let store = KeyFileStore::create(&path, "correct horse", FAST).unwrap();
    store.put(&SecretName::NodeKey, b"keep me").unwrap();

    assert!(matches!(
        KeyFileStore::create(&path, "other", FAST),
        Err(SecretError::Io { .. })
    ));
    let reopened = KeyFileStore::open(&path, "correct horse").unwrap();
    assert_eq!(
        reopened.get(&SecretName::NodeKey).unwrap(),
        Some(b"keep me".to_vec())
    );
}

#[test]
fn key_file_rejects_empty_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        KeyFileStore::create(dir.path().join("keys.oskf"), "", FAST),
        Err(SecretError::EmptyPassphrase)
    ));
}

#[test]
fn key_file_rejects_other_files_and_versions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys.oskf");
    write_private(
        &path,
        b"not a key file at all, just some text that is long enough",
    );
    assert!(matches!(
        KeyFileStore::open(&path, "x"),
        Err(SecretError::NotAKeyFile { .. })
    ));

    KeyFileStore::create(dir.path().join("real.oskf"), "x", FAST).unwrap();
    let mut bytes = std::fs::read(dir.path().join("real.oskf")).unwrap();
    bytes[4] = 2;
    std::fs::remove_file(&path).unwrap();
    write_private(&path, &bytes);
    assert!(matches!(
        KeyFileStore::open(&path, "x"),
        Err(SecretError::UnsupportedKeyFileVersion { version: 2, .. })
    ));
}

#[test]
fn key_file_rejects_absurd_argon2_parameters() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.oskf");
    KeyFileStore::create(&real, "x", FAST).unwrap();
    let mut bytes = std::fs::read(&real).unwrap();
    bytes[5..9].copy_from_slice(&u32::MAX.to_be_bytes()); // 4 TiB of memory
    let path = dir.path().join("hostile.oskf");
    write_private(&path, &bytes);
    assert!(matches!(
        KeyFileStore::open(&path, "x"),
        Err(SecretError::InvalidKeyFileParams(_))
    ));

    let too_many_lanes = KeyFileParams {
        parallelism: 64,
        ..FAST
    };
    assert!(matches!(
        KeyFileStore::create(dir.path().join("new.oskf"), "x", too_many_lanes),
        Err(SecretError::InvalidKeyFileParams(_))
    ));
}

#[test]
fn default_params_follow_rfc_9106() {
    let params = KeyFileParams::default();
    assert_eq!(params.memory_kib, 65_536);
    assert_eq!(params.iterations, 3);
    assert_eq!(params.parallelism, 4);
}

#[test]
fn debug_does_not_show_key_or_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let store = KeyFileStore::create(dir.path().join("keys.oskf"), "x", FAST).unwrap();
    store.put(&SecretName::NodeKey, b"super secret").unwrap();
    let printed = format!("{store:?}");
    assert!(!printed.contains("super secret"), "{printed}");
    assert!(!printed.contains("key:"), "{printed}");
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    std::io::Write::write_all(&mut file, bytes).unwrap();
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
}

#[cfg(unix)]
#[test]
fn key_file_is_private_and_refused_when_not() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys.oskf");
    let store = KeyFileStore::create(&path, "x", FAST).unwrap();
    store.put(&SecretName::NodeKey, b"v").unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        KeyFileStore::open(&path, "x"),
        Err(SecretError::InsecurePermissions { mode: 0o644, .. })
    ));
}

#[test]
fn key_file_leaves_no_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys.oskf");
    let store = KeyFileStore::create(&path, "x", FAST).unwrap();
    for i in 0..5u8 {
        store.put(&SecretName::NodeKey, &[i]).unwrap();
    }
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("keys.oskf")]);
}
