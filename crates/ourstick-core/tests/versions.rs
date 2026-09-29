//! A stick gets a new version (a new signed `seq`) only when its folder
//! changes; publishing an unchanged folder, e.g. after a restart, serves the
//! same record again.

mod common;

use common::Owner;
use ourstick_core::transport::Publisher;
use ourstick_core::Store;

#[tokio::test(flavor = "multi_thread")]
async fn unchanged_folder_keeps_its_version_and_a_change_makes_a_new_one() {
    let owner = Owner::new(&[("a.txt", b"hello"), ("b.txt", b"world")]).await;
    owner.clock.set(1_000);
    let first = owner.publish().await;
    assert!(first.new_version);
    assert_eq!(first.seq, 1_000);

    // Later, nothing changed: same record, same seq.
    owner.clock.set(5_000);
    let again = owner.publish().await;
    assert!(!again.new_version);
    assert_eq!(again.seq, first.seq);
    assert_eq!(again.record, first.record);
    let stick_id = owner.signing_key.stick_id();
    assert_eq!(
        owner
            .store
            .owned_stick(&stick_id)
            .unwrap()
            .unwrap()
            .last_signed_seq,
        1_000
    );

    // A file changes: a new version.
    std::fs::write(owner.folder.path().join("b.txt"), b"world, again").unwrap();
    let changed = owner.publish().await;
    assert!(changed.new_version);
    assert_eq!(changed.seq, 5_000);
    assert_ne!(changed.manifest_hash, first.manifest_hash);
    owner.publisher.shutdown().await;
}
