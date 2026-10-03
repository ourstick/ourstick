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
use ourstick_core::transport::{Fetcher, Publisher};

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
    let key = ReaderKey::generate();
    let sealed = seal(&report(), &key, &ticket.stick_id, &ticket.stick_key).unwrap();
    tokio::time::timeout(Duration::from_secs(20), fetcher.report(&ticket, &sealed))
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
    let elsewhere = ourstick_core::Ticket {
        stick_id: other,
        ..ticket.clone()
    };
    assert!(fetcher.report(&elsewhere, &sealed).await.is_err());
    // Too long: refused by the reader before it goes.
    assert!(fetcher
        .report(&ticket, &vec![1; MAX_REPORT_BYTES])
        .await
        .is_err());
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
    let connection = endpoint.connect(addr.clone(), REPORT_ALPN).await.unwrap();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    let request = [
        &ticket.stick_id.to_bytes()[..],
        &vec![1u8; MAX_REPORT_BYTES],
    ]
    .concat();
    // The owner stops reading at the limit and says so: never a clean end,
    // the sign of a report taken.
    let _ = send.write_all(&request).await;
    let _ = send.finish();
    assert!(recv.read_to_end(0).await.is_err());
    // The same peer's report of the right size is taken: the owner was
    // reached, and refused only the long one.
    let fine = seal(
        &report(),
        &ReaderKey::generate(),
        &ticket.stick_id,
        &ticket.stick_key,
    )
    .unwrap();
    let connection = endpoint.connect(addr.clone(), REPORT_ALPN).await.unwrap();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    send.write_all(&[&ticket.stick_id.to_bytes()[..], &fine].concat())
        .await
        .unwrap();
    send.finish().unwrap();
    recv.read_to_end(0).await.unwrap();
    assert_eq!(reports.recv().await.unwrap(), fine);
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
        fetcher.report(&ticket, &sealed).await.unwrap();
    }
    assert!(fetcher.report(&ticket, &sealed).await.is_err());
    fetcher.shutdown(false).await.unwrap();
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reader_tells_its_owner_it_has_all_of_it() {
    use ourstick_core::download::{download, DownloadEvent, ReceiveEvent, Reporter};
    use ourstick_core::store::{JoinedStick, Store};
    use std::ops::ControlFlow;
    use std::sync::Arc;

    let owner = Owner::new(&[("a.txt", b"hello"), ("b/c.txt", b"world")]).await;
    let mut reports = owner.publisher.take_reports().unwrap();
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let reader = Reader::new();
    reader
        .store
        .add_joined_stick(&JoinedStick::new(ticket.stick_id, reader.output.path(), 1))
        .unwrap();
    let reporter = Reporter::new(
        ticket.clone(),
        ReaderKey::generate(),
        reader.store.clone(),
        Arc::new(ourstick_core::clock::FixedClock::new(5_000)),
    );
    let fetcher = reader.fetcher().await;
    let total = std::sync::atomic::AtomicU64::new(0);
    let mut on_event = |event: DownloadEvent<'_>| {
        if let DownloadEvent::Planned { bytes } = event {
            total.store(bytes, std::sync::atomic::Ordering::SeqCst);
        }
        let planned = total.load(std::sync::atomic::Ordering::SeqCst);
        reporter.on_event(&ReceiveEvent::Download(event), 0, planned);
        ControlFlow::Continue(())
    };
    let downloaded = async {
        download(
            &fetcher,
            reader.store.clone(),
            &reader.clock,
            &ticket,
            reader.output.path(),
            &ourstick_core::download::RelayPolicy::default(),
            &mut on_event,
        )
        .await
        .unwrap();
        let planned = total.load(std::sync::atomic::Ordering::SeqCst);
        reporter.on_event(
            &ReceiveEvent::UpToDate {
                seq: publication.seq,
            },
            planned,
            planned,
        );
        // Until the owner has the last one.
        loop {
            let taken = tokio::time::timeout(Duration::from_secs(20), reports.recv())
                .await
                .expect("no report came")
                .unwrap();
            let opened = open(&taken, &ticket.stick_id, &ticket.stick_key).unwrap();
            if opened.report.state == ReportState::UpToDate {
                return opened.report;
            }
        }
    };
    let last = tokio::select! {
        last = downloaded => last,
        () = reporter.pump(&fetcher) => unreachable!(),
    };
    assert_eq!(last.seq, publication.seq);
    assert!(last.everything);
    fetcher.shutdown(false).await.unwrap();
    owner.publisher.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_report_goes_from_an_endpoint_with_no_blob_store() {
    use ourstick_core::download::ReportTo;
    use ourstick_core::transport::iroh::{IrohConfig, IrohReportSender, Network};

    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    let mut reports = owner.publisher.take_reports().unwrap();
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let sender =
        IrohReportSender::start(&IrohConfig::new(common::random_key(), Network::LocalOnly))
            .await
            .unwrap();
    let sealed = seal(
        &report(),
        &ReaderKey::generate(),
        &ticket.stick_id,
        &ticket.stick_key,
    )
    .unwrap();
    tokio::time::timeout(
        Duration::from_secs(20),
        ReportTo::report(&sender, &ticket, &sealed),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(reports.recv().await.unwrap(), sealed);
    sender.close().await;
    owner.publisher.shutdown().await;
}
