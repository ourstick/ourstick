//! The reader checks every received chunk against the hash it asked for, and
//! keeps nothing that does not match.
//!
//! An honest iroh-blobs store can never send such data: it validates its data
//! and outboard against the requested hash while sending (see `transfer.rs`).
//! So this test uses a lying provider that speaks the iroh-blobs get protocol
//! without any checks: asked for hash H, it answers with a well-formed bao
//! stream of *other* data.

use std::ops::ControlFlow;
use std::sync::Arc;

use bao_tree::io::outboard::PreOrderMemOutboard;
use bao_tree::{BlockSize, ChunkRanges};
use iroh::endpoint::{presets, Connection};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::Endpoint;
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, Network};
use ourstick_core::transport::{Fetcher, PeerConnection, TransportError};
use ourstick_core::{AddressHint, ManifestHint, NodeId, StickKey, StickSigningKey, Ticket};

/// iroh-blobs uses 16 KiB chunk groups (`IROH_BLOCK_SIZE`).
const IROH_BLOCK_SIZE: BlockSize = BlockSize::from_chunk_log(4);

/// Answers every get request with the same prepared response.
#[derive(Debug, Clone)]
struct LyingProvider {
    response: Arc<Vec<u8>>,
}

impl ProtocolHandler for LyingProvider {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let _request = recv
            .read_to_end(1024 * 1024)
            .await
            .map_err(AcceptError::from_err)?;
        send.write_all(&self.response)
            .await
            .map_err(AcceptError::from_err)?;
        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}

/// A get response as iroh-blobs sends it: size (u64 LE), then the bao encoding.
fn bao_response(data: &[u8]) -> Vec<u8> {
    let outboard = PreOrderMemOutboard::create(data, IROH_BLOCK_SIZE);
    let mut response = (data.len() as u64).to_le_bytes().to_vec();
    bao_tree::io::sync::encode_ranges_validated(
        data,
        &outboard,
        &ChunkRanges::all(),
        &mut response,
    )
    .unwrap();
    response
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_rejects_bytes_that_do_not_match_the_hash() {
    let honest: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let requested = *blake3::hash(&honest).as_bytes();
    let mut lie = honest.clone();
    lie[150_000] ^= 0x01;

    let provider = Router::builder(Endpoint::builder(presets::Minimal).bind().await.unwrap())
        .accept(
            iroh_blobs::ALPN,
            LyingProvider {
                response: Arc::new(bao_response(&lie)),
            },
        )
        .spawn();
    let addr = provider.endpoint().addr();
    let ticket = Ticket {
        stick_id: StickSigningKey::from_bytes(&[1; 32]).stick_id(),
        node_id: NodeId(*provider.endpoint().id().as_bytes()),
        manifest_hint: ManifestHint {
            seq: 1,
            hash: [0; 32],
        },
        address_hints: addr.ip_addrs().map(|ip| AddressHint::from(*ip)).collect(),
        stick_key: StickKey::from_bytes([2; 32]),
    };

    let store_dir = tempfile::tempdir().unwrap();
    let config = IrohConfig::new(
        StickKey::generate().as_bytes().to_owned(),
        Network::LocalOnly,
    );
    let fetcher = IrohFetcher::start(&config, store_dir.path()).await.unwrap();
    let connection = fetcher.connect(&ticket).await.unwrap();
    let result = connection
        .fetch_blob(&requested, &mut |_| ControlFlow::Continue(()))
        .await;

    assert!(matches!(result, Err(TransportError::Corrupt)), "{result:?}");
    assert_eq!(
        fetcher.local_bytes(&requested).await.unwrap(),
        0,
        "nothing unverified may be kept"
    );

    connection.close().await;
    fetcher.shutdown(false).await.unwrap();
    provider.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn honest_bao_response_is_accepted() {
    // The same lying provider with the true data: proves the test setup, not
    // just the damage, decides the outcome above.
    let honest: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let requested = *blake3::hash(&honest).as_bytes();

    let provider = Router::builder(Endpoint::builder(presets::Minimal).bind().await.unwrap())
        .accept(
            iroh_blobs::ALPN,
            LyingProvider {
                response: Arc::new(bao_response(&honest)),
            },
        )
        .spawn();
    let addr = provider.endpoint().addr();
    let ticket = Ticket {
        stick_id: StickSigningKey::from_bytes(&[1; 32]).stick_id(),
        node_id: NodeId(*provider.endpoint().id().as_bytes()),
        manifest_hint: ManifestHint {
            seq: 1,
            hash: [0; 32],
        },
        address_hints: addr.ip_addrs().map(|ip| AddressHint::from(*ip)).collect(),
        stick_key: StickKey::from_bytes([2; 32]),
    };

    let store_dir = tempfile::tempdir().unwrap();
    let config = IrohConfig::new(
        StickKey::generate().as_bytes().to_owned(),
        Network::LocalOnly,
    );
    let fetcher = IrohFetcher::start(&config, store_dir.path()).await.unwrap();
    let connection = fetcher.connect(&ticket).await.unwrap();
    let stats = connection
        .fetch_blob(&requested, &mut |_| ControlFlow::Continue(()))
        .await
        .unwrap();
    assert_eq!(stats.received, honest.len() as u64);
    assert_eq!(fetcher.read_blob(&requested).await.unwrap(), honest);

    connection.close().await;
    fetcher.shutdown(false).await.unwrap();
    provider.shutdown().await.unwrap();
}
