//! ADR 0007, the owner's pass: the catalogue in parts, partial versions as
//! it grows, a restart that goes on from what was recorded, and a record that
//! no longer matches its file.

mod common;

use std::time::Duration;

use common::{keep_going, Owner, Reader};
use ourstick_core::share::{run_pass, PartialTiming, PassEvent, PassOptions};
use ourstick_core::transport::{Fetcher, PeerConnection, Publisher, TransportError, GO};
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
        disks: common::disks(),
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
            // The stick's name travels with every version (ADR 0009).
            assert_eq!(publication.manifest.name, "test stick");
            partial.push(publication.manifest.entries.len());
        }
        PassEvent::Progress { done, total } => progress.push((done, total)),
        PassEvent::Roots { .. } => panic!("nothing is linked"),
        PassEvent::GivingWay(_) => {}
    })
    .await
    .unwrap();

    // An empty version at once (the stick gets its link), then one after
    // each part but the last; each is a prefix of the next.
    assert_eq!(partial, vec![0, 256, 512]);
    assert_eq!(progress, vec![(256, 600), (512, 600), (600, 600)]);
    let publication = outcome.publication;
    assert!(!publication.manifest.partial);
    assert_eq!(publication.manifest.name, "test stick");
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

    // A file added after every file the pass reads before its end: no
    // partial version would change anything, so none is published.
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

/// After a complete version, a partial one keeps the files not read yet as
/// they were: readers see new files early and none leaving.
#[tokio::test(flavor = "multi_thread")]
async fn a_later_pass_publishes_partial_versions_that_keep_the_rest() {
    let files = many_files();
    let owner = owner_of(&files).await;
    let first = run_pass(&publishing(&owner), PassOptions::default(), &mut |_| {})
        .await
        .unwrap()
        .publication;
    // Sorts before every other file, so the first part holds it.
    common::write_file(owner.folder.path(), "a-new.txt", b"new");
    std::fs::remove_file(owner.folder.path().join("f0599.txt")).unwrap();
    owner.clock.set(first.seq + 1_000);

    let mut published = Vec::new();
    let outcome = run_pass(&publishing(&owner), AT_ONCE, &mut |event| {
        if let PassEvent::Published(publication) = event {
            published.push(publication.manifest.clone());
        }
    })
    .await
    .unwrap();
    // One partial version, after the first part; the second part changes
    // nothing more, so it brings none.
    assert_eq!(published.len(), 1);
    let partial = &published[0];
    assert!(partial.partial);
    let mut expected: Vec<_> = first.manifest.entries[..599].to_vec();
    expected.insert(0, outcome.publication.manifest.entries[0].clone());
    assert_eq!(partial.entries, expected);
    assert_eq!(partial.entries[0].path, "a-new.txt");
    assert_eq!(outcome.publication.manifest.entries, expected);
    assert!(!outcome.publication.manifest.partial);
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
    // No empty version this time: the stick has a link already. The first
    // part only reads again what was recorded, so the next partial version
    // comes after the second.
    assert_eq!(published, vec![512]);
    assert_eq!(outcome.publication.reused, 256);
    assert_eq!(outcome.publication.manifest, first.manifest);
    assert!(outcome.publication.seq > first.seq + 1);
    owner.publisher.shutdown().await;
}

/// The invariant of ADR 0007: a blob is served only if its hash is the
/// manifest's. A record that no longer matches its file (here a hash no file
/// gives, with the stamp unchanged: the known limit) makes the pass list that
/// hash; the suitcase then gets another blob, which is never served, and the
/// record is forgotten, so the next pass reads the file and gets it right.
#[tokio::test(flavor = "multi_thread")]
async fn a_record_that_no_longer_matches_is_never_served_and_read_again() {
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
    let trusted = owner.publish().await;
    assert_eq!(trusted.manifest.entries[0].hash, [7; 32]);
    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let connection = fetcher.connect(&owner.ticket(&trusted)).await.unwrap();
    // Asked for: the keeper prepares it and finds another blob.
    let mut forgotten = false;
    for _ in 0..50 {
        let result = connection.fetch_blob(&[7; 32], &mut |_| GO).await;
        assert!(
            matches!(result, Err(TransportError::NotReady)),
            "{result:?}"
        );
        if owner
            .store
            .owned_files(&stick_id)
            .unwrap()
            .iter()
            .all(|file| file.path != "a.txt")
        {
            forgotten = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        forgotten,
        "the record that no longer matches is still there"
    );
    connection.close().await;
    fetcher.shutdown(false).await.unwrap();

    owner.clock.set(first.seq + 2_000);
    let again = run_pass(&publishing(&owner), PassOptions::default(), &mut |_| {})
        .await
        .unwrap();
    assert_eq!(again.publication.manifest, first.manifest);
    let report = Reader::new()
        .download(&owner.ticket(&again.publication), &mut keep_going)
        .await
        .unwrap();
    assert_eq!(report.files, 2);
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_unlinked_while_it_is_read_is_neither_recorded_nor_published() {
    // A linked folder of more files than a part holds, read first ("Films"
    // sorts before "a.txt").
    let elsewhere = tempfile::tempdir().unwrap();
    let films = elsewhere.path().join("Films");
    for index in 0..400 {
        common::write_file(&films, &format!("{index:03}.mkv"), b"film");
    }
    let owner = Owner::new(&[("a.txt", b"a")]).await;
    let stick_id = owner.signing_key.stick_id();
    let entry = ourstick_core::linked::plan_link(
        owner.store.as_ref(),
        &stick_id,
        owner.cache.path(),
        &[],
        &films,
        1,
    )
    .unwrap();
    owner.store.add_linked_entry(&stick_id, &entry).unwrap();

    // Removed once the first part is recorded, as the window's Remove does.
    let store = owner.store.clone();
    let mut published = Vec::new();
    let ended = run_pass(&publishing(&owner), AT_ONCE, &mut |event| match event {
        PassEvent::Progress { done, .. } if done > 0 => {
            if !store.linked_entries(&stick_id).unwrap().is_empty() {
                store.remove_linked_entry(&stick_id, "Films").unwrap();
            }
        }
        PassEvent::Published(publication) => published.push(publication.manifest.clone()),
        _ => {}
    })
    .await;
    assert!(
        matches!(ended, Err(ourstick_core::share::ShareError::LayoutChanged)),
        "{ended:?}"
    );
    // Nothing of it was published after it was removed, nor recorded again.
    for manifest in published.iter().skip(1) {
        assert!(
            manifest
                .entries
                .iter()
                .all(|entry| !entry.path.starts_with("Films/")),
            "a version after the removal has Films"
        );
    }
    assert!(owner
        .store
        .owned_files(&stick_id)
        .unwrap()
        .iter()
        .all(|file| !file.path.starts_with("Films/")));

    // The next pass reads the stick as it is now.
    let outcome = run_pass(&publishing(&owner), PassOptions::default(), &mut |_| {})
        .await
        .unwrap();
    let paths: Vec<&str> = outcome
        .publication
        .manifest
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(paths, ["a.txt"]);
    owner.publisher.shutdown().await;
}
