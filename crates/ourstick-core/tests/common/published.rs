//! Sticks published into the in-memory transport, for tests.

use std::path::Path;
use std::sync::{Arc, Mutex};

use ourstick_core::clock::FixedClock;
use ourstick_core::share::publish_folder;
use ourstick_core::transport::Publisher;
use ourstick_core::{
    cipher_for, encrypt_blob, CipherId, Manifest, ManifestHint, MemoryStore, OwnedStick, StickKey,
    StickSigningKey, Store, Ticket,
};

use super::memory::{MemoryPublisher, Served};

fn random_key() -> [u8; 32] {
    use rand::RngCore;
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    key
}

/// Puts every file's blob in the owner's store, ready: these tests are about
/// the reader, and the owner's own store has tests of its own.
pub fn make_ready(
    served: &Mutex<Served>,
    folder: &Path,
    stick_key: &StickKey,
    manifest: &Manifest,
) {
    let cipher = cipher_for(CipherId::XChaCha20Poly1305, stick_key);
    let mut served = served.lock().unwrap();
    for entry in &manifest.entries {
        let on_disk = entry
            .path
            .split('/')
            .fold(folder.to_path_buf(), |path, part| path.join(part));
        let mut blob = Vec::new();
        encrypt_blob(
            cipher.as_ref(),
            &mut std::io::Cursor::new(std::fs::read(on_disk).unwrap()),
            &mut blob,
        )
        .unwrap();
        assert_eq!(*blake3::hash(&blob).as_bytes(), entry.hash);
        served.blobs.insert(entry.hash, blob);
    }
}

/// A stick with one file of `size` bytes, published into memory.
pub struct Published {
    pub served: Arc<Mutex<Served>>,
    pub ticket: Ticket,
    pub data: Vec<u8>,
    pub manifest: Manifest,
    pub signing_key: StickSigningKey,
    _folder: tempfile::TempDir,
}

pub async fn publish_one_file(size: usize) -> Published {
    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    let mut published = publish_files(&[("movie.bin", &data)]).await;
    published.data = data;
    published
}

/// A stick with these files (path, contents), published into memory.
/// `data` is left empty.
pub async fn publish_files(files: &[(&str, &[u8])]) -> Published {
    let folder = tempfile::tempdir().unwrap();
    for (path, contents) in files {
        let path = folder.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
    let data = Vec::new();
    let signing_key = StickSigningKey::from_bytes(&random_key());
    let stick_key = StickKey::from_bytes(random_key());
    let owner_store = Arc::new(MemoryStore::new());
    owner_store
        .add_owned_stick(&OwnedStick::new(
            signing_key.stick_id(),
            "test",
            folder.path(),
            CipherId::XChaCha20Poly1305,
            1,
        ))
        .unwrap();
    let served = Arc::new(Mutex::new(Served::default()));
    let publisher = MemoryPublisher {
        served: served.clone(),
        staging: Arc::new(tempfile::tempdir().unwrap()),
    };
    let publication = publish_folder(
        &publisher,
        owner_store,
        &FixedClock::new(1_000),
        &signing_key,
        &stick_key,
        false,
    )
    .await
    .unwrap();
    make_ready(&served, folder.path(), &stick_key, &publication.manifest);
    let ticket = Ticket {
        stick_id: signing_key.stick_id(),
        node_id: publisher.node_id(),
        manifest_hint: ManifestHint {
            seq: publication.seq,
            hash: publication.manifest_hash,
        },
        address_hints: Vec::new(),
        stick_key,
    };
    Published {
        served,
        ticket,
        data,
        manifest: publication.manifest,
        signing_key,
        _folder: folder,
    }
}

/// A stick published in several versions, one after the other; `served`
/// starts at the first, and `versions[i]` is what the owner serves at
/// version `i` (swap it in to publish that version).
pub struct Versions {
    pub served: Arc<Mutex<Served>>,
    pub versions: Vec<Served>,
    pub ticket: Ticket,
    _folder: tempfile::TempDir,
}

impl Versions {
    /// The owner now serves version `index`, and only its blobs.
    pub fn publish(&self, index: usize) {
        *self.served.lock().unwrap() = self.versions[index].clone();
    }

    /// The owner serves nothing at all for a while (e.g. it is offline).
    pub fn withdraw(&self) {
        *self.served.lock().unwrap() = Served::default();
    }

    /// The owner keeps its record but stops serving every blob.
    pub fn stop_serving_blobs(&self) {
        self.served.lock().unwrap().allowed.clear();
    }
}

/// Publishes each list of files as the next version of one stick, at seq
/// 1000, 2000, …
pub async fn publish_versions(versions: &[&[(&str, &[u8])]]) -> Versions {
    let folder = tempfile::tempdir().unwrap();
    let signing_key = StickSigningKey::from_bytes(&random_key());
    let stick_key = StickKey::from_bytes(random_key());
    let owner_store = Arc::new(MemoryStore::new());
    owner_store
        .add_owned_stick(&OwnedStick::new(
            signing_key.stick_id(),
            "test",
            folder.path(),
            CipherId::XChaCha20Poly1305,
            1,
        ))
        .unwrap();
    let served = Arc::new(Mutex::new(Served::default()));
    let publisher = MemoryPublisher {
        served: served.clone(),
        staging: Arc::new(tempfile::tempdir().unwrap()),
    };
    let mut states = Vec::new();
    let mut first = None;
    for (index, files) in versions.iter().enumerate() {
        for entry in std::fs::read_dir(folder.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                std::fs::remove_dir_all(path).unwrap();
            } else {
                std::fs::remove_file(path).unwrap();
            }
        }
        for (path, content) in *files {
            let path = folder.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let publication = publish_folder(
            &publisher,
            owner_store.clone(),
            &FixedClock::new(1_000 * (index as u64 + 1)),
            &signing_key,
            &stick_key,
            true,
        )
        .await
        .unwrap();
        make_ready(&served, folder.path(), &stick_key, &publication.manifest);
        first.get_or_insert(publication);
        states.push(served.lock().unwrap().clone());
    }
    let first = first.unwrap();
    let ticket = Ticket {
        stick_id: signing_key.stick_id(),
        node_id: publisher.node_id(),
        manifest_hint: ManifestHint {
            seq: first.seq,
            hash: first.manifest_hash,
        },
        address_hints: Vec::new(),
        stick_key,
    };
    let versions = Versions {
        served,
        versions: states,
        ticket,
        _folder: folder,
    };
    versions.publish(0);
    versions
}
