//! ADR 0015 with two iroh peers on this machine: a reader's sealed report
//! reaches the owner's side whole, and opens there with the stick's key;
//! a request longer than a report is never handed on.

mod common;

use std::time::Duration;

use common::{Owner, Reader};
use iroh::endpoint::presets;
use iroh::Endpoint;
use ourstick_core::report::{
    open, seal, ComputerKind, ReaderKey, ReaderReport, ReportState, MAX_REPORT_BYTES,
};
use ourstick_core::transport::iroh::REPORT_ALPN;
use ourstick_core::transport::{Fetcher, PeerConnection, Publisher};

fn report() -> ReaderReport {
    ReaderReport {
        counter: 1,
        seq: 1,
        state: ReportState::UpToDate,
        everything: true,
        kind: ComputerKind::Linux,
        name: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_reaches_the_owner_and_opens_with_the_sticks_key() {
    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    let mut reports = owner.publisher.take_reports().unwrap();
    assert!(owner.publisher.take_reports().is_none());
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let connection = fetcher.connect(&ticket).await.unwrap();
    let key = ReaderKey::generate();
    let sealed = seal(&report(), &key, &ticket.stick_id, &ticket.stick_key).unwrap();
    tokio::time::timeout(
        Duration::from_secs(20),
        connection.report(&ticket.stick_id, &sealed),
    )
    .await
    .expect("the report was not taken in time")
    .unwrap();
    let taken = tokio::time::timeout(Duration::from_secs(5), reports.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(taken, sealed);
    let opened = open(&taken, &ticket.stick_id, &ticket.stick_key).unwrap();
    assert_eq!(opened.reader, key.reader_id());
    assert_eq!(opened.report, report());
    // One for another stick is not handed on.
    let other = ourstick_core::StickSigningKey::from_bytes(&[8; 32]).stick_id();
    // The reader is told it was not taken, so it keeps it.
    assert!(connection.report(&other, &sealed).await.is_err());
    // Too long: refused by the reader before it goes.
    assert!(connection
        .report(&ticket.stick_id, &vec![1; MAX_REPORT_BYTES])
        .await
        .is_err());
    connection.close().await;
    fetcher.shutdown(false).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(500), reports.recv())
            .await
            .is_err(),
        "a report for another stick was handed on"
    );
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_longer_than_a_report_is_not_read_past_the_limit() {
    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    let mut reports = owner.publisher.take_reports().unwrap();
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    // A peer that does not keep the rules: its own endpoint, a raw stream.
    let endpoint = Endpoint::builder(presets::Minimal).bind().await.unwrap();
    let mut addr = iroh::EndpointAddr::new(iroh::PublicKey::from_bytes(&ticket.node_id.0).unwrap());
    for hint in &ticket.address_hints {
        match hint {
            ourstick_core::AddressHint::Ipv4 { ip, port } => {
                addr = addr.with_ip_addr(std::net::SocketAddr::from((*ip, *port)));
            }
            ourstick_core::AddressHint::Ipv6 { ip, port } => {
                addr = addr.with_ip_addr(std::net::SocketAddr::from((*ip, *port)));
            }
            ourstick_core::AddressHint::Relay { .. } => {}
        }
    }
    let connection = endpoint.connect(addr, REPORT_ALPN).await.unwrap();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    let request = [
        &ticket.stick_id.to_bytes()[..],
        &vec![1u8; MAX_REPORT_BYTES],
    ]
    .concat();
    // The owner stops reading at the limit and answers nothing.
    let _ = send.write_all(&request).await;
    let _ = send.finish();
    assert!(recv.read_to_end(0).await.is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(500), reports.recv())
            .await
            .is_err(),
        "a request past the limit was handed on"
    );
    endpoint.close().await;
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_not_taken_is_told_so() {
    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let connection = fetcher.connect(&ticket).await.unwrap();
    let sealed = seal(
        &report(),
        &ReaderKey::generate(),
        &ticket.stick_id,
        &ticket.stick_key,
    )
    .unwrap();
    // Never opened: 32 wait (also before the owner's side takes them), the
    // next is not taken.
    for _ in 0..32 {
        connection.report(&ticket.stick_id, &sealed).await.unwrap();
    }
    assert!(connection.report(&ticket.stick_id, &sealed).await.is_err());
    connection.close().await;
    fetcher.shutdown(false).await.unwrap();
    owner.publisher.shutdown().await;
}
