//! ADR 0007, the owner's side: a blob of the current manifest that is not in
//! the cache yet is "not ready", never a failure, and the cache is not emptied
//! after a restart until a full pass has said what it still needs.

mod common;

use std::collections::HashSet;
use std::time::Duration;

use common::{noise, random_key, Owner, Reader};
use ourstick_core::transport::iroh::{IrohConfig, IrohPublisher, Network};
use ourstick_core::transport::{Fetcher, PeerConnection, Publisher, TransportError, GO};

#[tokio::test(flavor = "multi_thread")]
async fn an_allowed_blob_not_in_the_cache_is_not_ready_until_imported() {
    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);

    // A blob the owner will prepare later: allowed now, imported below.
    let later = noise(40_000, 5);
    let later_hash = *blake3::hash(&later).as_bytes();
    let mut allowed: HashSet<_> = publication
        .manifest
        .entries
        .iter()
        .map(|e| e.hash)
        .collect();
    allowed.insert(later_hash);
    owner
        .publisher
        .publish(publication.record.clone(), allowed)
        .await
        .unwrap();

    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let connection = fetcher.connect(&ticket).await.unwrap();
    let result = connection.fetch_blob(&later_hash, &mut |_| GO).await;
    assert!(
        matches!(result, Err(TransportError::NotReady)),
        "{result:?}"
    );
    // Not allowed stays refused.
    let result = connection.fetch_blob(&[9; 32], &mut |_| GO).await;
    assert!(matches!(result, Err(TransportError::Refused)), "{result:?}");

    let file = owner.publisher.staging_dir().join("later.blob");
    std::fs::write(&file, &later).unwrap();
    assert_eq!(
        owner.publisher.import_blob(&file).await.unwrap(),
        later_hash
    );
    let stats = connection
        .fetch_blob(&later_hash, &mut |_| GO)
        .await
        .unwrap();
    assert_eq!(stats.received, later.len() as u64);

    connection.close().await;
    fetcher.shutdown(false).await.unwrap();
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restarted_owner_keeps_its_cache_until_it_says_what_to_keep() {
    let big = noise(100_000, 6);
    let owner = Owner::new(&[("big.bin", &big)]).await;
    let publication = owner.publish().await;
    let big_hash = publication.manifest.entries[0].hash;
    owner.publisher.shutdown().await;

    let config = IrohConfig::new(random_key(), Network::LocalOnly)
        .with_gc_interval(Duration::from_millis(100));
    let publisher = IrohPublisher::start(&config, owner.signing_key.stick_id(), owner.cache.path())
        .await
        .unwrap();
    // A first, small version (as a partial one would be): only the manifest.
    assert!(publisher
        .keep_blob(&publication.manifest_hash)
        .await
        .unwrap());
    publisher
        .publish(publication.record.clone(), HashSet::new())
        .await
        .unwrap();
    // Several GC intervals: the unprotected file blob is still there.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(publisher.has_blob(&big_hash).await.unwrap());

    // Once told what to keep, the rest goes at the next GC run.
    publisher.keep_only_blobs(HashSet::new()).await.unwrap();
    let mut gone = false;
    for _ in 0..50 {
        if !publisher.has_blob(&big_hash).await.unwrap() {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(gone, "the blob nobody keeps is still in the cache");
    assert!(publisher
        .has_blob(&publication.manifest_hash)
        .await
        .unwrap());
    publisher.shutdown().await;
}
