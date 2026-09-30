//! A peer must not be able to write into the owner's cache, nor read from it
//! except by a plain get of an allowed blob. The publisher refuses every
//! other request explicitly (ADR 0005). This test pushes like an attacker would.

mod common;

use common::{noise, Owner, Reader};
use iroh::endpoint::presets;
use iroh::Endpoint;
use iroh_blobs::protocol::{ChunkRangesSeq, GetManyRequest, PushRequest};
use iroh_blobs::store::mem::MemStore;
use iroh_blobs::Hash;
use ourstick_core::transport::{Fetcher, PeerConnection, Publisher, TransportError, GO};
use std::net::SocketAddr;

#[tokio::test(flavor = "multi_thread")]
async fn owner_refuses_pushed_blobs() {
    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);

    // The attacker holds some data and tries to push it into the owner's cache.
    let attacker_store = MemStore::new();
    let data = noise(50_000, 9);
    let hash = Hash::new(&data);
    let _tag = attacker_store.add_bytes(data).temp_tag().await.unwrap();
    let attacker = Endpoint::builder(presets::Minimal).bind().await.unwrap();

    let owner_key = iroh::PublicKey::from_bytes(&ticket.node_id.0).unwrap();
    let mut owner_addr = iroh::EndpointAddr::new(owner_key);
    for hint in &ticket.address_hints {
        if let ourstick_core::AddressHint::Ipv4 { ip, port } = hint {
            owner_addr = owner_addr.with_ip_addr(SocketAddr::from((*ip, *port)));
        }
    }
    let connection = attacker
        .connect(owner_addr, iroh_blobs::ALPN)
        .await
        .unwrap();
    let pushed = attacker_store
        .remote()
        .execute_push(connection, PushRequest::new(hash, ChunkRangesSeq::root()))
        .await;

    // What matters: nothing was sent, and no one can get the blob from the
    // owner.
    if let Ok(stats) = &pushed {
        assert_eq!(stats.payload_bytes_written, 0, "no payload may be sent");
    }
    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let asking = fetcher.connect(&ticket).await.unwrap();
    let result = asking.fetch_blob(hash.as_bytes(), &mut |_| GO).await;
    assert!(matches!(result, Err(TransportError::Refused)), "{result:?}");
    asking.close().await;
    fetcher.shutdown(false).await.unwrap();

    attacker.close().await;
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_refuses_get_many_even_for_allowed_blobs() {
    let owner = Owner::new(&[("a.bin", &noise(50_000, 10))]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let allowed = Hash::from_bytes(publication.manifest.entries[0].hash);

    let reader_store = MemStore::new();
    let reader = Endpoint::builder(presets::Minimal).bind().await.unwrap();
    let owner_key = iroh::PublicKey::from_bytes(&ticket.node_id.0).unwrap();
    let mut owner_addr = iroh::EndpointAddr::new(owner_key);
    for hint in &ticket.address_hints {
        if let ourstick_core::AddressHint::Ipv4 { ip, port } = hint {
            owner_addr = owner_addr.with_ip_addr(SocketAddr::from((*ip, *port)));
        }
    }
    let connection = reader.connect(owner_addr, iroh_blobs::ALPN).await.unwrap();
    let result = reader_store
        .remote()
        .execute_get_many(
            connection,
            GetManyRequest::new(vec![allowed], ChunkRangesSeq::all()),
        )
        .await;

    assert!(result.is_err(), "get-many should be refused: {result:?}");
    assert!(!reader_store.has(allowed).await.unwrap());
    reader.close().await;
    owner.publisher.shutdown().await;
}
