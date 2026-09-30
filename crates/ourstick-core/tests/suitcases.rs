//! ADR 0007, the owner's suitcases on iroh: a blob is served from a suitcase
//! only if its hash is the expected one; a suitcase leaves only once no
//! transfer from it is in progress; what readers ask for reaches the owner.

mod common;

use std::collections::HashSet;
use std::ops::ControlFlow;
use std::time::Duration;

use common::{noise, Owner, Reader};
use ourstick_core::transport::{Demand, Fetcher, PeerConnection, Publisher, TransportError, GO};

/// An owner whose current version also allows `data`'s blob, not in any
/// store yet; returns the blob's hash.
async fn owner_allowing(data: &[u8]) -> (Owner, ourstick_core::Ticket, [u8; 32]) {
    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    let publication = owner.publish().await;
    let hash = *blake3::hash(data).as_bytes();
    let mut allowed: HashSet<_> = publication
        .manifest
        .entries
        .iter()
        .map(|e| e.hash)
        .collect();
    allowed.insert(hash);
    owner
        .publisher
        .publish(publication.record.clone(), allowed)
        .await
        .unwrap();
    let ticket = owner.ticket(&publication);
    (owner, ticket, hash)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_is_served_from_a_suitcase_only_with_the_expected_hash() {
    let data = noise(300_000, 1);
    let (owner, ticket, hash) = owner_allowing(&data).await;
    let mut demand = owner.publisher.take_demand().unwrap();
    assert!(owner.publisher.take_demand().is_none(), "only one taker");

    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let connection = fetcher.connect(&ticket).await.unwrap();
    let result = connection.fetch_blob(&hash, &mut |_| GO).await;
    assert!(
        matches!(result, Err(TransportError::NotReady)),
        "{result:?}"
    );
    let Some(Demand::Asked {
        hash: asked,
        served: false,
        ..
    }) = demand.recv().await
    else {
        panic!("the ask did not reach the owner");
    };
    assert_eq!(asked, hash);

    let (suitcase, folder) = owner.publisher.open_suitcase().await.unwrap();
    // The wrong bytes under that name: never served.
    let wrong = folder.join("wrong.blob");
    std::fs::write(&wrong, b"not the blob").unwrap();
    assert!(!owner
        .publisher
        .add_to_suitcase(suitcase, &wrong, &hash)
        .await
        .unwrap());
    let result = connection.fetch_blob(&hash, &mut |_| GO).await;
    assert!(
        matches!(result, Err(TransportError::NotReady)),
        "{result:?}"
    );
    // A file outside the suitcase's folder is not taken.
    let outside = owner.cache.path().join("outside.blob");
    std::fs::write(&outside, &data).unwrap();
    assert!(owner
        .publisher
        .add_to_suitcase(suitcase, &outside, &hash)
        .await
        .is_err());

    let right = folder.join("right.blob");
    std::fs::write(&right, &data).unwrap();
    assert!(owner
        .publisher
        .add_to_suitcase(suitcase, &right, &hash)
        .await
        .unwrap());
    let stats = connection.fetch_blob(&hash, &mut |_| GO).await.unwrap();
    assert_eq!(stats.received, data.len() as u64);
    assert_eq!(fetcher.read_blob(&hash).await.unwrap(), data);

    // Evicted: its folder goes, and the blob is not ready any more.
    owner.publisher.evict(suitcase).await.unwrap();
    assert!(!folder.exists());
    // Another reader, which holds nothing of it.
    let second = Reader::new();
    let again = second.fetcher().await;
    let other = again.connect(&ticket).await.unwrap();
    let result = other.fetch_blob(&hash, &mut |_| GO).await;
    assert!(
        matches!(result, Err(TransportError::NotReady)),
        "{result:?}"
    );

    connection.close().await;
    other.close().await;
    fetcher.shutdown(false).await.unwrap();
    again.shutdown(false).await.unwrap();
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_suitcase_leaves_only_once_its_transfer_ended() {
    let data = noise(2_000_000, 2);
    let (owner, ticket, hash) = owner_allowing(&data).await;
    let (suitcase, folder) = owner.publisher.open_suitcase().await.unwrap();
    let file = folder.join("blob");
    std::fs::write(&file, &data).unwrap();
    assert!(owner
        .publisher
        .add_to_suitcase(suitcase, &file, &hash)
        .await
        .unwrap());

    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let connection = fetcher.connect(&ticket).await.unwrap();
    let (started, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
    // Slowly, so the eviction comes while the transfer is in progress.
    let mut slowly = |_| {
        started.send(()).ok();
        ControlFlow::Continue(Duration::from_millis(20))
    };
    let transfer = connection.fetch_blob(&hash, &mut slowly);
    let evict = async {
        started_rx.recv().await;
        owner.publisher.evict(suitcase).await.unwrap();
    };
    let (stats, ()) = tokio::join!(transfer, evict);
    // Had the store closed under it, the transfer would have failed.
    let stats = stats.expect("the transfer in progress was cut by the eviction");
    assert_eq!(stats.received, data.len() as u64);
    assert!(!folder.exists());

    connection.close().await;
    fetcher.shutdown(false).await.unwrap();
    owner.publisher.shutdown().await;
}
