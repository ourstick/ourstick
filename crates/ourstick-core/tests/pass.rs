//! ADR 0007, the owner's pass: the catalogue in parts with the cache filled
//! behind it, partial versions while a stick has no complete one, and a
//! restart that goes on from what was recorded.

mod common;

use std::time::Duration;

use common::{keep_going, Owner, Reader};
use ourstick_core::share::{run_pass, PartialTiming, PassEvent, PassOptions};
use ourstick_core::transport::Publisher;
use ourstick_core::watcher::Owner as Publishing;
use ourstick_core::Store;

/// A partial version after every part.
const AT_ONCE: PassOptions = PassOptions {
    rescan: false,
    partial: Some(PartialTiming {
        first_after: Duration::ZERO,
        every: Duration::ZERO,
    }),
};

/// More files than two parts hold.
fn many_files() -> Vec<(String, Vec<u8>)> {
    (0..600)
        .map(|i| (format!("f{i:04}.txt"), format!("file {i}").into_bytes()))
        .collect()
}

async fn owner_of(files: &[(String, Vec<u8>)]) -> Owner {
    let files: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(path, data)| (path.as_str(), data.as_slice()))
        .collect();
    Owner::new(&files).await
}

fn publishing(owner: &Owner) -> Publishing<'_, ourstick_core::transport::iroh::IrohPublisher> {
    Publishing {
        publisher: &owner.publisher,
        store: owner.store.clone(),
        clock: &owner.clock,
        signing_key: &owner.signing_key,
        stick_key: &owner.stick_key,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_share_is_published_as_its_catalogue_grows() {
    let files = many_files();
    let owner = owner_of(&files).await;
    let mut partial = Vec::new();
    let mut progress = Vec::new();
    let outcome = run_pass(&publishing(&owner), AT_ONCE, &mut |event| match event {
        PassEvent::Published(publication) => {
            assert!(publication.manifest.partial);
            partial.push(publication.manifest.entries.len());
        }
        PassEvent::Progress { done, total } => progress.push((done, total)),
    })
    .await
    .unwrap();

    // An empty version at once (the stick gets its link), then one after
    // each part but the last; each is a prefix of the next.
    assert_eq!(partial, vec![0, 256, 512]);
    assert_eq!(progress, vec![(256, 600), (512, 600), (600, 600)]);
    let publication = outcome.publication;
    assert!(!publication.manifest.partial);
    assert_eq!(publication.manifest.entries.len(), 600);
    assert_eq!(outcome.skipped, 0);
    let stick = owner
        .store
        .owned_stick(&owner.signing_key.stick_id())
        .unwrap()
        .unwrap();
    assert!(!stick.latest_partial);
    assert_eq!(stick.last_signed_seq, publication.seq);

    // A reader holding the first, empty version's link gets everything.
    let reader = Reader::new();
    let report = reader
        .download(&owner.ticket(&publication), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(report.files, 600);
    assert_eq!(reader.read("f0599.txt"), b"file 599");

    // Once a complete version exists, a pass publishes no partial ones.
    common::write_file(owner.folder.path(), "new.txt", b"new");
    owner.clock.set(publication.seq + 1_000);
    let mut more = 0;
    let outcome = run_pass(&publishing(&owner), AT_ONCE, &mut |event| {
        more += usize::from(matches!(event, PassEvent::Published(_)));
    })
    .await
    .unwrap();
    assert_eq!(more, 0);
    assert_eq!(outcome.publication.manifest.entries.len(), 601);
    assert_eq!(outcome.publication.reused, 600);
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restarted_first_share_goes_on_from_what_it_recorded() {
    let files = many_files();
    let owner = owner_of(&files).await;
    let first = run_pass(&publishing(&owner), PassOptions::default(), &mut |_| {})
        .await
        .unwrap()
        .publication;
    let stick_id = owner.signing_key.stick_id();
    let recorded = owner.store.owned_files(&stick_id).unwrap();
    assert_eq!(recorded.len(), 600);
    assert!(recorded.iter().all(|file| file.plaintext_hash.is_some()));

    // As if the first share had stopped after 256 files: only those are
    // recorded, and the latest version is a partial one.
    let paths: Vec<String> = recorded[..256]
        .iter()
        .map(|file| file.path.clone())
        .collect();
    owner
        .store
        .keep_only_owned_files(&stick_id, &paths)
        .unwrap();
    let partial = owner.signing_key.sign_record(first.seq + 1, [7; 32]);
    owner.store.save_signed_record(&partial, true).unwrap();
    owner.clock.set(first.seq + 10);

    let mut published = Vec::new();
    let outcome = run_pass(&publishing(&owner), AT_ONCE, &mut |event| {
        if let PassEvent::Published(publication) = event {
            published.push(publication.manifest.entries.len());
        }
    })
    .await
    .unwrap();
    // No empty version this time: the stick has a link already.
    assert_eq!(published, vec![256, 512]);
    assert_eq!(outcome.publication.reused, 256);
    assert_eq!(outcome.publication.manifest, first.manifest);
    assert!(outcome.publication.seq > first.seq + 1);
    owner.publisher.shutdown().await;
}

/// The invariant of ADR 0007: a blob enters the cache only if its hash is the
/// catalogue's. A record that no longer matches its file (here a hash no
/// file gives, with the stamp unchanged) leaves the file out of the version,
/// and the next pass reads it again.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_whose_blob_is_not_the_catalogued_one_is_left_for_the_next_pass() {
    let owner = Owner::new(&[("a.txt", b"one"), ("b.txt", b"two")]).await;
    let first = owner.publish().await;
    let stick_id = owner.signing_key.stick_id();
    let mut recorded = owner.store.owned_files(&stick_id).unwrap();
    recorded[0].blob_hash = [7; 32];
    owner
        .store
        .record_owned_files(&stick_id, &recorded)
        .unwrap();

    owner.clock.set(first.seq + 1_000);
    let outcome = run_pass(&publishing(&owner), PassOptions::default(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(outcome.skipped, 1);
    let paths: Vec<_> = outcome
        .publication
        .manifest
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(paths, vec!["b.txt"]);
    assert!(!owner.publisher.has_blob(&[7; 32]).await.unwrap());

    owner.clock.set(first.seq + 2_000);
    let again = run_pass(&publishing(&owner), PassOptions::default(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(again.skipped, 0);
    assert_eq!(again.publication.manifest, first.manifest);
    owner.publisher.shutdown().await;
}
