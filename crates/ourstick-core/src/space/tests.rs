use std::collections::HashMap;

use super::*;
use crate::locks::{download_lock, lock_for_sharing};
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
    assert_eq!(reserve(256 * GB), 20 * GB);
    assert_eq!(reserve(30 * GB), 3 * GB);
    // ADR 0008's examples.
    assert_eq!(room_left(60 * GB, 256 * GB, 6 * GB), 34 * GB);
    assert_eq!(room_left(10 * GB, 30 * GB, 2 * GB), 5 * GB);
    assert_eq!(room_left(GB, 30 * GB, 0), 0);
}

#[test]
fn one_sharer_gets_what_it_always_got() {
    assert_eq!(cache_share(161 * GB, 0, 1).unwrap(), 161 * GB / 10);
    assert_eq!(cache_share(5 * GB, 0, 1).unwrap(), MIN_BUDGET);
    assert_eq!(cache_share(483 * GB, 0, 1).unwrap(), MAX_BUDGET);
    assert!(matches!(
        cache_share(GB, 0, 1),
        Err(SpaceError::NoRoomForCache { .. })
    ));
}

#[test]
fn sharers_split_one_budget_but_each_gets_two_gigabytes() {
    // 100 GB free, another share holds 10: the device's budget is 11, so
    // only the floor is left for this one.
    assert_eq!(cache_share(100 * GB, 10 * GB, 1).unwrap(), MIN_BUDGET);
    // 300 GB free, others hold 10: the budget is 31, 21 are left.
    assert_eq!(
        cache_share(300 * GB, 10 * GB, 1).unwrap(),
        31 * GB - 10 * GB
    );
    // Not even the floor is free.
    assert!(matches!(
        cache_share(GB, 10 * GB, 1),
        Err(SpaceError::NoRoomForCache { .. })
    ));
}

#[test]
fn a_cache_claim_counts_while_its_stick_is_shared() {
    let (store, data) = setup(2);
    let disks = FakeDisks(HashMap::from([
        ("data", (100 * GB, 500 * GB)),
        ("s", (10 * GB, 50 * GB)),
    ]));
    let cache = Path::new("/data/cache");
    let (first, _) = lock_for_sharing(data.path(), &store, &stick_id(1)).unwrap();
    let mine = claim_cache(&disks, &store, data.path(), &stick_id(1), cache, 1).unwrap();
    assert_eq!(mine, 10 * GB);
    // What is left on the cache's disk counts it whole.
    let left = available(&disks, &store, data.path(), cache, None).unwrap();
    assert_eq!(left, 100 * GB - 20 * GB - 10 * GB);
    // A second sharer gets what is left of the device's budget.
    let (_second, _) = lock_for_sharing(data.path(), &store, &stick_id(2)).unwrap();
    let theirs = claim_cache(&disks, &store, data.path(), &stick_id(2), cache, 1).unwrap();
    assert_eq!(theirs, MIN_BUDGET);
    // The first one stops (or crashes): its claim no longer counts.
    drop(first);
    let left = available(&disks, &store, data.path(), cache, None).unwrap();
    assert_eq!(left, 100 * GB - 20 * GB - theirs);
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
            assert_eq!(available, 100 * GB - 20 * GB - 60 * GB);
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
        (&*Box::leak(real.into_boxed_str()), (12 * GB, 100 * GB)),
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

#[test]
fn sticks_started_together_get_equal_parts() {
    // 300 GB free: a device budget of 30 GB, three sticks at once.
    let first = cache_share(300 * GB, 0, 3).unwrap();
    assert_eq!(first, 10 * GB);
    let second = cache_share(300 * GB - first, first, 3).unwrap();
    assert_eq!(second, 10 * GB);
    let third = cache_share(300 * GB - 2 * first, 2 * first, 3).unwrap();
    assert_eq!(third, 10 * GB);
    // Many on a small disk: each still gets the floor while it is free.
    assert_eq!(cache_share(30 * GB, 0, 10).unwrap(), MIN_BUDGET);
}
