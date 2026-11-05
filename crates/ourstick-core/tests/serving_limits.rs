//! A peer with the link opens many gets of a file and never reads them
//! (ADR 0031): the owner serves at most so many at once, so they cannot hold
//! its open files (macOS gives an app 256), and an honest reader still gets
//! the file afterwards.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use common::{fetch_when_ready, noise, Owner, Reader};
use iroh::endpoint::{presets, Connection, RecvStream, SendStream};
use iroh::Endpoint;
use iroh_blobs::protocol::{GetRequest, Request};
use ourstick_core::transport::{Fetcher, PeerConnection, Publisher, GO};
use ourstick_core::{AddressHint, Ticket};

#[cfg(target_os = "linux")]
fn open_files() -> usize {
    std::fs::read_dir("/proc/self/fd").unwrap().count()
}

fn owner_addr(ticket: &Ticket) -> iroh::EndpointAddr {
    let key = iroh::PublicKey::from_bytes(&ticket.node_id.0).unwrap();
    let mut addr = iroh::EndpointAddr::new(key);
    for hint in &ticket.address_hints {
        if let AddressHint::Ipv4 { ip, port } = hint {
            addr = addr.with_ip_addr(SocketAddr::from((*ip, *port)));
        }
    }
    addr
}

/// Opens up to `streams` gets of `hash` on `connection`, never read; stops
/// when the owner takes no more streams (its QUIC limit).
async fn stalled_gets(
    connection: &Connection,
    hash: [u8; 32],
    streams: usize,
) -> Vec<(SendStream, RecvStream)> {
    let request: Request = GetRequest::blob(iroh_blobs::Hash::from_bytes(hash)).into();
    let bytes = postcard::to_stdvec(&request).unwrap();
    let mut held = Vec::new();
    for _ in 0..streams {
        let Ok(Ok((mut send, receive))) =
            tokio::time::timeout(Duration::from_secs(2), connection.open_bi()).await
        else {
            break;
        };
        send.write_all(&bytes).await.unwrap();
        send.finish().unwrap();
        held.push((send, receive));
    }
    held
}

#[tokio::test(flavor = "multi_thread")]
async fn stalled_gets_hold_only_so_many_files() {
    let data = noise(8 * 1024 * 1024, 3);
    let owner = Owner::new(&[("big.bin", &data)]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let hash = publication.manifest.entries[0].hash;

    // Made ready by an honest reader first.
    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let connection = fetcher.connect(&ticket).await.unwrap();
    fetch_when_ready(&connection, &hash, &mut |_| GO)
        .await
        .unwrap();
    connection.close().await;
    fetcher.shutdown(false).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    #[cfg(target_os = "linux")]
    let before = open_files();
    let attacker = Endpoint::builder(presets::Minimal).bind().await.unwrap();
    let mut connections = Vec::new();
    let mut held = Vec::new();
    for _ in 0..4 {
        let connection = attacker
            .connect(owner_addr(&ticket), iroh_blobs::ALPN)
            .await
            .unwrap();
        held.extend(stalled_gets(&connection, hash, 100).await);
        connections.push(connection);
    }
    // Streams past the program's places were refused at once, and so freed
    // their place in QUIC for more: all were asked.
    assert!(held.len() > 2 * 64, "{} streams open", held.len());
    tokio::time::sleep(Duration::from_secs(5)).await;
    #[cfg(target_os = "linux")]
    {
        // Two files per transfer (the file and its hash tree), at most 64
        // transfers in the whole program; 132 stalled gets held 264 before.
        let grown = open_files().saturating_sub(before);
        assert!(grown < 64 * 2 + 40, "{grown} more files open");
    }

    // An honest reader is told "not ready" while all places are taken, and
    // gets the file once they are free.
    drop(held);
    for connection in connections {
        connection.close(0u32.into(), b"done");
    }
    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let connection = fetcher.connect(&ticket).await.unwrap();
    fetch_when_ready(&connection, &hash, &mut |_| GO)
        .await
        .unwrap();
    assert!(fetcher.local_bytes(&hash).await.unwrap() > 0);
    connection.close().await;
    fetcher.shutdown(false).await.unwrap();
    owner.publisher.shutdown().await;
}

/// Opens one stream on `connection` and sends a byte of a request, never
/// the rest: the owner waits for it (at most 10 s, ADR 0031).
async fn silent_stream(connection: &Connection) -> (SendStream, RecvStream) {
    let (mut send, receive) = connection.open_bi().await.unwrap();
    send.write_all(&[0]).await.unwrap();
    (send, receive)
}

#[tokio::test(flavor = "multi_thread")]
async fn silent_streams_never_keep_a_reader_out() {
    let data = noise(64 * 1024, 4);
    let owner = Owner::new(&[("a.bin", &data)]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let hash = publication.manifest.entries[0].hash;

    // Every connection a stick holds, each with a stream that asks nothing:
    // a node id is enough, no key needed.
    let attacker = Endpoint::builder(presets::Minimal).bind().await.unwrap();
    let mut connections = Vec::new();
    let mut held = Vec::new();
    for _ in 0..128 {
        let connection = attacker
            .connect(owner_addr(&ticket), iroh_blobs::ALPN)
            .await
            .unwrap();
        held.push(silent_stream(&connection).await);
        connections.push(connection);
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    // An honest reader still gets the file, well before the silent streams
    // would end by themselves.
    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let fetched = tokio::time::timeout(Duration::from_secs(8), async {
        let connection = fetcher.connect(&ticket).await.unwrap();
        fetch_when_ready(&connection, &hash, &mut |_| GO)
            .await
            .unwrap();
        connection.close().await;
    })
    .await;
    assert!(fetched.is_ok(), "the reader was kept out");
    assert!(fetcher.local_bytes(&hash).await.unwrap() > 0);
    drop(held);
    for connection in connections {
        connection.close(0u32.into(), b"done");
    }
    fetcher.shutdown(false).await.unwrap();
    owner.publisher.shutdown().await;
}
