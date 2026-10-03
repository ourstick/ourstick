//! ADR 0007, a restarted owner: its latest version is served again at once,
//! from the manifest kept for it, and its files come from suitcases prepared
//! anew, before any pass reads the folder.

mod common;

use std::time::Duration;

use common::{keep_going, noise, random_key, Owner, Reader};
use ourstick_core::share::{keep_suitcases, serve_latest, Budget};
use ourstick_core::transport::iroh::{IrohConfig, IrohPublisher, Network};
use ourstick_core::transport::Publisher;
use ourstick_core::{ManifestHint, Ticket};

/// After a restart the latest version is served again at once, from the
/// manifest kept for it, before any pass reads the folder; the suitcases of
/// the run before are gone and prepared again when asked for.
#[tokio::test(flavor = "multi_thread")]
async fn a_restarted_owner_serves_its_latest_version_at_once() {
    let owner = Owner::new(&[("a.txt", b"hello"), ("b.bin", &noise(50_000, 8))]).await;
    let publication = owner.publish().await;
    owner.publisher.shutdown().await;

    let config = IrohConfig::new(random_key(), Network::LocalOnly);
    let publisher = IrohPublisher::start(&config, owner.signing_key.stick_id(), owner.cache.path())
        .await
        .unwrap();
    tokio::spawn(keep_suitcases(
        publisher.clone(),
        owner.store.clone(),
        owner.signing_key.stick_id(),
        owner.stick_key.clone(),
        Budget::new(1 << 30),
        common::disks(),
    ));
    let publishing = ourstick_core::share::Owner {
        publisher: &publisher,
        store: owner.store.clone(),
        disks: common::disks(),
        clock: &owner.clock,
        signing_key: &owner.signing_key,
        stick_key: &owner.stick_key,
    };
    let served = serve_latest(&publishing).await.unwrap().unwrap();
    assert_eq!(served.seq, publication.seq);
    assert_eq!(served.manifest, publication.manifest);
    assert!(!served.new_version);

    let ticket = Ticket {
        stick_id: owner.signing_key.stick_id(),
        node_id: publisher.node_id(),
        manifest_hint: ManifestHint {
            seq: served.seq,
            hash: served.manifest_hash,
        },
        address_hints: publisher.address_hints(),
        stick_key: owner.stick_key.clone(),
    };
    // Every blob the cache holds is served at once: a reader never waits on
    // "not ready" for them (which would never end on its own).
    let reader = Reader::new();
    let report = tokio::time::timeout(
        Duration::from_secs(60),
        reader.download(&ticket, &mut keep_going),
    )
    .await
    .expect("the download waited on blobs the cache holds")
    .unwrap();
    assert_eq!(report.files, 2);
    assert_eq!(reader.read("a.txt"), b"hello");
    publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stick_never_published_has_no_latest_version() {
    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    let publishing = ourstick_core::share::Owner {
        publisher: &owner.publisher,
        store: owner.store.clone(),
        disks: common::disks(),
        clock: &owner.clock,
        signing_key: &owner.signing_key,
        stick_key: &owner.stick_key,
    };
    assert!(serve_latest(&publishing).await.unwrap().is_none());
    owner.publisher.shutdown().await;
}
