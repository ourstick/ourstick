use std::collections::HashMap;

use super::*;
use crate::locks::download_lock;
use crate::stick_id::StickSigningKey;
use crate::store::{MemoryStore, OwnedStick};

const GB: u64 = 1 << 30;

/// Disks named by the first component of a path: `/a/...` is disk "a".
struct FakeDisks(HashMap<&'static str, (u64, u64)>);

impl FakeDisks {
    fn name(path: &Path) -> String {
        path.components()
            .nth(1)
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn get(&self, path: &Path) -> (u64, u64) {
        self.0[Self::name(path).as_str()]
    }
}

impl Disks for FakeDisks {
    fn key(&self, path: &Path) -> std::io::Result<String> {
        Ok(Self::name(path))
    }
    fn free(&self, path: &Path) -> std::io::Result<u64> {
        Ok(self.get(path).0)
    }
    fn total(&self, path: &Path) -> std::io::Result<u64> {
        Ok(self.get(path).1)
    }
}

fn stick_id(seed: u8) -> StickId {
    StickSigningKey::from_bytes(&[seed; 32]).stick_id()
}

/// A store with owned sticks 1..=count, and a data folder for the locks.
fn setup(count: u8) -> (MemoryStore, tempfile::TempDir) {
    let store = MemoryStore::new();
    for seed in 1..=count {
        store
            .add_owned_stick(&OwnedStick {
                node_id: Some(crate::ticket::NodeId([seed; 32])),
                ..OwnedStick::new(
                    stick_id(seed),
                    format!("s{seed}"),
                    format!("/s/{seed}"),
                    crate::cipher::CipherId::XChaCha20Poly1305,
                    1,
                )
            })
            .unwrap();
    }
    (store, tempfile::tempdir().unwrap())
}

#[test]
fn the_reserve_and_what_is_left() {
    assert_eq!(reserve(256 * GB), 2 * GB);
    assert_eq!(reserve(30 * GB), 3 * GB / 2);
    assert_eq!(reserve(8 * GB), 8 * GB / 20);
    // ADR 0008's examples, with the reserve of its amendment of 2026-11-06.
    assert_eq!(room_left(60 * GB, 256 * GB, 6 * GB), 52 * GB);
    assert_eq!(room_left(10 * GB, 30 * GB, 2 * GB), 13 * GB / 2);
    assert_eq!(room_left(GB, 30 * GB, 0), 0);
    // The Mac that took nothing with a tenth kept back: 228 GB, 13 GB free.
    assert_eq!(room_left(13 * GB, 228 * GB, 0), 11 * GB);
}

#[test]
fn two_downloads_that_fit_alone_do_not_both_start() {
    let (store, data) = setup(2);
    let disks = FakeDisks(HashMap::from([
        ("out", (100 * GB, 200 * GB)),
        ("data", (50 * GB, 100 * GB)),
    ]));
    let out = Path::new("/out/films");
    let data_dir = data.path();
    // The fake's data disk is named by the real temporary folder's first
    // component; give it the "data" figures under that name too.
    let mut disks = disks;
    let real = FakeDisks::name(data_dir);
    let figures = disks.0["data"];
    disks.0.insert(Box::leak(real.into_boxed_str()), figures);

    let _one = download_lock(data_dir, &stick_id(1)).unwrap();
    claim_download(&disks, &store, data_dir, &stick_id(1), out, 60 * GB, GB).unwrap();
    let _two = download_lock(data_dir, &stick_id(2)).unwrap();
    match claim_download(&disks, &store, data_dir, &stick_id(2), out, 60 * GB, GB) {
        Err(SpaceError::DoesNotFit { needed, available }) => {
            assert_eq!(needed, 60 * GB);
            assert_eq!(available, 100 * GB - 2 * GB - 60 * GB);
        }
        other => panic!("{other:?}"),
    }
    // As files land, it holds less.
    store
        .update_space_claim(&stick_id(1), ClaimKind::Download, 10 * GB)
        .unwrap();
    claim_download(&disks, &store, data_dir, &stick_id(2), out, 60 * GB, GB).unwrap();
}

#[test]
fn a_download_needs_room_for_its_batches_too() {
    let (store, data) = setup(1);
    let data_dir = data.path();
    let real = FakeDisks::name(data_dir);
    // The data disk has 2 GB left after its reserve; out has plenty.
    let disks = FakeDisks(HashMap::from([
        ("out", (100 * GB, 200 * GB)),
        (&*Box::leak(real.into_boxed_str()), (4 * GB, 100 * GB)),
    ]));
    let _lock = download_lock(data_dir, &stick_id(1)).unwrap();
    assert!(matches!(
        claim_download(&disks, &store, data_dir, &stick_id(1), Path::new("/out/x"), GB, 3 * GB),
        Err(SpaceError::DoesNotFit { needed, .. }) if needed == 3 * GB
    ));
    claim_download(
        &disks,
        &store,
        data_dir,
        &stick_id(1),
        Path::new("/out/x"),
        GB,
        2 * GB,
    )
    .unwrap();
}

#[test]
fn the_real_disks_answer() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("not/yet");
    let disks = SystemDisks;
    assert_eq!(disks.key(&missing).unwrap(), disks.key(dir.path()).unwrap());
    assert!(disks.total(&missing).unwrap() >= disks.free(&missing).unwrap());
}

fn data_disk(data_dir: &Path, free: u64, total: u64) -> FakeDisks {
    let real = FakeDisks::name(data_dir);
    FakeDisks(HashMap::from([
        ("out", (500 * GB, 1000 * GB)),
        (&*Box::leak(real.into_boxed_str()), (free, total)),
    ]))
}

#[test]
fn each_download_holds_its_own_batch_room() {
    let (store, data) = setup(3);
    let data_dir = data.path();
    // 10 GB left on the data disk after its reserve.
    let disks = data_disk(data_dir, 12 * GB, 100 * GB);
    let out = Path::new("/out/x");
    let _one = download_lock(data_dir, &stick_id(1)).unwrap();
    claim_download(&disks, &store, data_dir, &stick_id(1), out, GB, 4 * GB).unwrap();
    let _two = download_lock(data_dir, &stick_id(2)).unwrap();
    claim_download(&disks, &store, data_dir, &stick_id(2), out, GB, 4 * GB).unwrap();
    let _three = download_lock(data_dir, &stick_id(3)).unwrap();
    match claim_download(&disks, &store, data_dir, &stick_id(3), out, GB, 4 * GB) {
        Err(SpaceError::DoesNotFit { needed, available }) => {
            assert_eq!((needed, available), (4 * GB, 2 * GB));
        }
        other => panic!("{other:?}"),
    }
    // Each holds two rows: its bytes on the folder's disk, its batches on
    // the data folder's.
    let rows = store.space_claims().unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows
        .iter()
        .any(|claim| claim.kind == ClaimKind::Scratch && claim.bytes == 4 * GB));
}

#[test]
fn a_nearly_full_laptop_still_takes_a_small_stick() {
    // 100 000 small files, 465 MB: refused on 2026-11-06 by a Mac of 228 GB
    // with 13 GB free, when a tenth of the disk was kept back.
    let (store, data) = setup(1);
    let data_dir = data.path();
    let real = FakeDisks::name(data_dir);
    let disks = FakeDisks(HashMap::from([(
        &*Box::leak(real.into_boxed_str()),
        (13 * GB, 228 * GB),
    )]));
    let (needed, largest) = (465_000_000, 9_300);
    let _lock = download_lock(data_dir, &stick_id(1)).unwrap();
    claim_download(
        &disks,
        &store,
        data_dir,
        &stick_id(1),
        &data_dir.join("Downloads/Measurement"),
        needed + largest,
        crate::download::scratch_bytes(needed, largest),
    )
    .unwrap();
}
