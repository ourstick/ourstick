//! A peer must not be able to write into the owner's cache. The publisher refuses every
//! other request explicitly (ADR 0005). This test pushes like an attacker would.

mod common;

use common::{noise, Owner};
use iroh::endpoint::presets;
use iroh::Endpoint;
use iroh_blobs::protocol::{ChunkRangesSeq, PushRequest};
use iroh_blobs::store::mem::MemStore;
use iroh_blobs::Hash;
use ourstick_core::transport::Publisher;
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

    // What matters: the owner's cache.
    if let Ok(stats) = &pushed {
        assert_eq!(stats.payload_bytes_written, 0, "no payload may be sent");
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let stored = owner.publisher.has_blob(hash.as_bytes()).await.unwrap();
    assert!(!stored, "the pushed blob must not be in the owner's cache");

    attacker.close().await;
    owner.publisher.shutdown().await;
}
