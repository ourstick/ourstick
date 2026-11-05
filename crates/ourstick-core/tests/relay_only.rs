//! A reader that cannot reach the owner directly (a firewall that drops all
//! UDP, both sides behind strict NATs): it still gets the stick, through
//! the public relays, and the owner, after 30 seconds of such a reader,
//! says that readers reach it only through the relay (the firewall advice
//! of the app). Needs internet, so it is ignored by default. Run with:
//! `cargo test -p ourstick-core --test relay_only -- --ignored --nocapture`

mod common;

use std::ops::ControlFlow;
use std::time::Duration;

use common::{noise, random_key, Owner, Reader};
use ourstick_core::download::{download, DownloadEvent, RelayPolicy};
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, Network};
use ourstick_core::transport::{Fetcher, PathKind, PeerConnection, Publisher};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs internet access to the public iroh relays"]
async fn a_reader_with_no_direct_path_gets_the_stick_through_the_relay() {
    let data = noise(1_500_000, 6);
    let owner = Owner::on(Network::Public, &[("a.bin", &data), ("b.txt", b"hello")]).await;
    let publication = owner.publish().await;
    assert!(
        owner.publisher.online(Duration::from_secs(30)).await,
        "no home relay within 30 s"
    );
    let ticket = owner.ticket(&publication);

    let reader = Reader::on(Network::Public);
    let config = IrohConfig::new(random_key(), Network::Public).without_direct_paths();
    let fetcher = IrohFetcher::start(&config, reader.blobs.path())
        .await
        .unwrap();
    let mut paths = Vec::new();
    let started = std::time::Instant::now();
    let report = download(
        &fetcher,
        reader.store.clone(),
        &reader.clock,
        &ticket,
        reader.output.path(),
        &RelayPolicy::default(),
        &mut |event| {
            if let DownloadEvent::Connected { path } | DownloadEvent::PathChanged { path } = event {
                println!("path: {path}");
                paths.push(path);
            }
            ControlFlow::Continue(())
        },
    )
    .await
    .unwrap();
    println!(
        "received {} bytes in {:.1} s: {} direct, {} via relay",
        report.received,
        started.elapsed().as_secs_f64(),
        report.usage.direct,
        report.usage.relay
    );
    assert_eq!(report.files, 2);
    assert_eq!(reader.read("a.bin"), data);
    assert_eq!(report.usage.direct, 0, "a direct path with no IP transport");
    assert!(report.usage.relay >= report.received);
    assert!(
        paths.iter().all(|path| *path == PathKind::Relay),
        "{paths:?}"
    );

    // A reader that stays (connected, through the relay) past 30 s: the
    // owner sees that readers reach it only through the relay.
    let mut activity = owner.publisher.activity();
    assert!(!activity.borrow().only_relayed, "told before 30 s");
    let connection = fetcher.connect(&ticket).await.unwrap();
    let told = tokio::time::timeout(
        Duration::from_secs(60),
        activity.wait_for(|activity| activity.only_relayed),
    )
    .await;
    assert!(
        told.is_ok(),
        "the owner never said readers come only through the relay"
    );
    connection.close().await;
    fetcher.shutdown(false).await.unwrap();
    owner.publisher.shutdown().await;
}
