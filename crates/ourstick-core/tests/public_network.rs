//! Transfer over the public iroh network (n0 relays and address lookup), as the
//! CLI does. Needs internet, so it is ignored by default. Run with:
//! `cargo test -p ourstick-core --test public_network -- --ignored --nocapture`

mod common;

use std::ops::ControlFlow;
use std::time::Duration;

use common::{noise, Owner, Reader};
use ourstick_core::download::DownloadEvent;
use ourstick_core::transport::iroh::Network;
use ourstick_core::transport::{PathKind, Publisher};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs internet access to the public iroh relays"]
async fn transfer_over_public_network() {
    let big = noise(2_000_000, 5);
    let owner = Owner::on(Network::Public, &[("big.bin", &big), ("a.txt", b"hello")]).await;
    let publication = owner.publish().await;
    assert!(
        owner.publisher.online(Duration::from_secs(30)).await,
        "no home relay within 30 s"
    );
    let ticket = owner.ticket(&publication);
    // The network report is measured in the background; give it a moment.
    for _ in 0..20 {
        if owner.publisher.network_report().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    match owner.publisher.network_report() {
        Some(report) => println!(
            "owner network: {}",
            ourstick_core::transport::describe_network(&report)
        ),
        None => println!("owner network: not measured yet"),
    }
    assert!(
        ticket
            .address_hints
            .iter()
            .any(|hint| matches!(hint, ourstick_core::AddressHint::Relay { .. })),
        "the ticket should name the owner's relay"
    );
    println!(
        "ticket link has {} characters",
        ticket.to_link().unwrap().len()
    );

    let reader = Reader::on(Network::Public);
    let mut paths = Vec::new();
    let report = reader
        .download(&ticket, &mut |event| {
            match event {
                DownloadEvent::Connected { path } | DownloadEvent::PathChanged { path } => {
                    println!("path: {path}");
                    paths.push(path);
                }
                _ => {}
            }
            ControlFlow::Continue(())
        })
        .await
        .unwrap();
    println!(
        "received {} bytes: {} direct, {} via relay",
        report.received, report.usage.direct, report.usage.relay
    );
    assert!(report.usage.direct + report.usage.relay >= report.received);
    assert_eq!(report.files, 2);
    assert_eq!(reader.read("big.bin"), big);
    assert!(paths.iter().all(|path| *path != PathKind::Unknown) || paths.len() > 1);
    owner.publisher.shutdown().await;
}
