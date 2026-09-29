//! A stick gets a new version (a new signed `seq`) only when its folder
//! changes; publishing an unchanged folder, e.g. after a restart, serves the
//! same record again.

mod common;

use common::{keep_going, Owner, Reader};
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

#[tokio::test(flavor = "multi_thread")]
async fn only_changed_files_are_encrypted_again_and_readers_get_the_new_content() {
    let owner = Owner::new(&[("a.txt", b"one"), ("b.txt", b"two"), ("c.txt", b"three")]).await;
    let first = owner.publish().await;
    assert_eq!(first.reused, 0);

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(owner.folder.path().join("b.txt"), b"two, changed").unwrap();
    owner.clock.set(10_000);
    let second = owner.publish().await;
    assert!(second.new_version);
    assert_eq!(second.reused, 2, "a.txt and c.txt kept their blobs");

    let reader = Reader::new();
    reader
        .download(&owner.ticket(&second), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(reader.read("a.txt"), b"one");
    assert_eq!(reader.read("b.txt"), b"two, changed");
    assert_eq!(reader.read("c.txt"), b"three");
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rescan_reads_every_file_whatever_was_recorded() {
    let owner = Owner::new(&[("a.txt", b"one"), ("b.txt", b"two")]).await;
    let first = owner.publish().await;
    let stick_id = owner.signing_key.stick_id();

    // The known limit: a record that no longer describes the file, while the
    // stamp still matches, is trusted. Simulated by swapping two blob hashes.
    let mut recorded = owner.store.owned_files(&stick_id).unwrap();
    let (a, b) = (recorded[0].blob_hash, recorded[1].blob_hash);
    recorded[0].blob_hash = b;
    recorded[1].blob_hash = a;
    owner
        .store
        .replace_owned_files(&stick_id, &recorded)
        .unwrap();
    owner.clock.set(10_000);
    let trusted = owner.publish().await;
    assert_ne!(trusted.manifest_hash, first.manifest_hash);

    // --rescan reads the files and gets the true manifest back.
    owner.clock.set(20_000);
    let rescanned = owner.publish_with(true).await;
    assert_eq!(rescanned.reused, 0);
    assert_eq!(rescanned.manifest, first.manifest);
    owner.publisher.shutdown().await;
}
