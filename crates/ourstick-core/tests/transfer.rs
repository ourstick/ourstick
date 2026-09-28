//! The most important test file in the repository (CLAUDE.md rule 5): two peers
//! on one machine transfer a folder, resume after an interruption, reject a
//! damaged blob and a stale record, and the owner refuses what it must refuse.

mod common;

use std::ops::ControlFlow;
use std::time::{Duration, UNIX_EPOCH};

use common::{keep_going, noise, write_file, Owner, Reader};
use ourstick_core::download::{DownloadError, DownloadEvent};
use ourstick_core::transport::{Fetcher, PathKind, PeerConnection, Publisher, TransportError};
use ourstick_core::{CurrentManifest, JoinedStick, RecordError, Store};

#[tokio::test(flavor = "multi_thread")]
async fn full_transfer_of_a_folder() {
    let big = noise(300_000, 1);
    let owner = Owner::new(&[
        ("a.txt", b"hello"),
        ("sub/deep/big.bin", &big),
        ("empty", b""),
        ("copy-of-a.txt", b"hello"),
    ])
    .await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);

    let reader = Reader::new();
    let mut path = None;
    let report = reader
        .download(&ticket, &mut |event| {
            if let DownloadEvent::Connected { path: kind } = event {
                path = Some(kind);
            }
            ControlFlow::Continue(())
        })
        .await
        .unwrap();

    assert_eq!(path, Some(PathKind::Direct));
    // Without relays everything arrives directly, and at least the payload does.
    assert_eq!(report.usage.relay, 0);
    assert!(report.usage.direct >= report.received, "{:?}", report.usage);
    assert_eq!(report.files, 4);
    assert_eq!(report.bytes, 300_010);
    assert_eq!(reader.read("a.txt"), b"hello");
    assert_eq!(reader.read("copy-of-a.txt"), b"hello");
    assert_eq!(reader.read("sub/deep/big.bin"), big);
    assert_eq!(reader.read("empty"), b"");
    assert!(!reader.exists(".ourstick-partial"));

    // Modification times are carried over (whole seconds).
    let source = std::fs::metadata(owner.folder.path().join("a.txt")).unwrap();
    let copy = std::fs::metadata(reader.output.path().join("a.txt")).unwrap();
    let seconds = |time: std::time::SystemTime| time.duration_since(UNIX_EPOCH).unwrap().as_secs();
    assert_eq!(
        seconds(copy.modified().unwrap()),
        seconds(source.modified().unwrap())
    );

    // The accepted record is stored; running again is a no-op resume, not an error.
    let joined = reader
        .store
        .joined_stick(&ticket.stick_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        joined.current_manifest.unwrap().hash,
        publication.manifest_hash
    );
    let again = reader.download(&ticket, &mut keep_going).await.unwrap();
    assert_eq!(again.received, 0);

    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupted_download_resumes_where_it_stopped() {
    let big = noise(8 * 1024 * 1024, 2);
    let owner = Owner::new(&[("big.bin", &big)]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let hash = publication.manifest.entries[0].hash;
    let reader = Reader::new();

    // Cancel once a good part has arrived. (iroh-blobs writes received data to
    // its store in batches, so cancelling after the very first chunk may leave
    // nothing to resume; that is not an error, the next run just starts over.)
    let interrupted = reader
        .download(&ticket, &mut |event| match event {
            DownloadEvent::Progress { received, .. } if received > 2 * 1024 * 1024 => {
                ControlFlow::Break(())
            }
            _ => ControlFlow::Continue(()),
        })
        .await;
    assert!(matches!(interrupted, Err(DownloadError::Cancelled)));
    assert!(
        !reader.exists("big.bin"),
        "no half-written file under its final name"
    );

    let fetcher = reader.fetcher().await;
    let partial = fetcher.local_bytes(&hash).await.unwrap();
    fetcher.shutdown(true).await.unwrap();
    let blob_len = ourstick_core::blob_len(
        ourstick_core::cipher_for(ourstick_core::CipherId::XChaCha20Poly1305, &owner.stick_key)
            .as_ref(),
        big.len() as u64,
    );
    assert!(partial > 0 && partial < blob_len, "partial = {partial}");

    let resumed = reader.download(&ticket, &mut keep_going).await.unwrap();
    assert!(resumed.received > 0);
    assert!(
        resumed.received <= blob_len - partial,
        "only the missing part is fetched again: {} of {blob_len}",
        resumed.received
    );
    assert_eq!(reader.read("big.bin"), big);
    owner.publisher.shutdown().await;
}

/// A damaged blob in the owner's cache never reaches the reader: iroh-blobs
/// checks every chunk against its outboard while sending and stops. The reader
/// gets an error that says so, and writes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn damaged_blob_is_rejected_and_nothing_is_written() {
    let owner = Owner::new(&[("big.bin", &noise(300_000, 3))]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);

    // Flip one byte of the encrypted blob in the owner's cache.
    let data_file = owner.cached_data_file(&publication.manifest.entries[0].hash);
    let mut bytes = std::fs::read(&data_file).unwrap();
    bytes[150_000] ^= 0x01;
    std::fs::write(&data_file, bytes).unwrap();

    let reader = Reader::new();
    let result = reader.download(&ticket, &mut keep_going).await;
    assert!(
        matches!(
            result,
            Err(DownloadError::Transport(TransportError::OwnerFailed))
        ),
        "{result:?}"
    );
    assert!(!reader.exists("big.bin"));
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn record_older_than_what_the_reader_has_is_rejected() {
    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    owner.clock.set(500);
    let publication = owner.publish().await; // seq 500
    let ticket = owner.ticket(&publication);

    let reader = Reader::new();
    let mut joined = JoinedStick::new(ticket.stick_id, reader.output.path(), 1);
    joined.current_manifest = Some(CurrentManifest {
        seq: 1_000,
        hash: [7; 32],
    });
    reader.store.add_joined_stick(&joined).unwrap();

    let result = reader.download(&ticket, &mut keep_going).await;
    assert!(
        matches!(
            result,
            Err(DownloadError::Record(RecordError::NotNewer {
                seq: 500,
                last_seen: 1_000
            }))
        ),
        "{result:?}"
    );
    assert!(!reader.exists("a.txt"));
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn record_older_than_the_ticket_is_rejected() {
    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    owner.clock.set(500);
    let publication = owner.publish().await;
    let mut ticket = owner.ticket(&publication);
    ticket.manifest_hint.seq = 600; // as if the ticket came from a later version

    let reader = Reader::new();
    let result = reader.download(&ticket, &mut keep_going).await;
    assert!(
        matches!(
            result,
            Err(DownloadError::Record(RecordError::OlderThanTicket { .. }))
        ),
        "{result:?}"
    );
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_serves_only_blobs_of_the_current_manifest() {
    let removed = noise(100_000, 4);
    let owner = Owner::new(&[("keep.txt", b"keep"), ("removed.bin", &removed)]).await;
    let first = owner.publish().await;
    let removed_hash = first
        .manifest
        .entries
        .iter()
        .find(|entry| entry.path == "removed.bin")
        .unwrap()
        .hash;

    std::fs::remove_file(owner.folder.path().join("removed.bin")).unwrap();
    write_file(owner.folder.path(), "new.txt", b"new");
    owner.clock.set(2_000);
    let second = owner.publish().await;
    let ticket = owner.ticket(&second);

    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let connection = fetcher.connect(&ticket).await.unwrap();
    for hash in [removed_hash, [9; 32]] {
        let result = connection
            .fetch_blob(&hash, &mut |_| ControlFlow::Continue(()))
            .await;
        assert!(matches!(result, Err(TransportError::Refused)), "{result:?}");
    }
    connection.close().await;
    fetcher.shutdown(false).await.unwrap();

    // The removed blob leaves the cache at the next GC run; kept ones stay.
    let mut gone = false;
    for _ in 0..50 {
        if !owner.publisher.has_blob(&removed_hash).await.unwrap() {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(gone, "removed blob is still in the cache");
    for entry in &second.manifest.entries {
        assert!(owner.publisher.has_blob(&entry.hash).await.unwrap());
    }
    assert!(owner
        .publisher
        .has_blob(&second.manifest_hash)
        .await
        .unwrap());

    let report = reader.download(&ticket, &mut keep_going).await.unwrap();
    assert_eq!(report.files, 2);
    assert!(!reader.exists("removed.bin"));
    owner.publisher.shutdown().await;
}

/// Names that are valid on the owner's disk but unsafe for a reader: a Windows
/// drive-like name, and our own partial directory in another letter case. The
/// reader refuses the manifest before writing anything or accepting its record.
#[tokio::test(flavor = "multi_thread")]
async fn reader_refuses_unsafe_names_before_writing_anything() {
    for (name, contents) in [
        ("c:evil", &b"escape"[..]),
        (".OURSTICK-PARTIAL/x", &b"hidden"[..]),
    ] {
        let owner = Owner::new(&[("ok.txt", b"fine"), (name, contents)]).await;
        let publication = owner.publish().await;
        let ticket = owner.ticket(&publication);

        let reader = Reader::new();
        let result = reader.download(&ticket, &mut keep_going).await;
        assert!(
            matches!(result, Err(DownloadError::UnsafePath { .. })),
            "{name}: {result:?}"
        );
        assert!(!reader.exists("ok.txt"), "{name}: wrote files anyway");
        let joined = reader
            .store
            .joined_stick(&ticket.stick_id)
            .unwrap()
            .unwrap();
        assert_eq!(joined.current_manifest, None, "{name}: record was accepted");
        owner.publisher.shutdown().await;
    }
}
