//! `share` waits for the network measurement before printing "This network",
//! but never longer than it asked: with no relays (local only) the measurement
//! does not finish, and the wait must end on time.

mod common;

use std::time::{Duration, Instant};

use common::Owner;
use ourstick_core::transport::Publisher;

#[tokio::test(flavor = "multi_thread")]
async fn waiting_for_the_network_report_ends_on_time() {
    let owner = Owner::new(&[("a.txt", b"hello")]).await;
    let wait = Duration::from_millis(500);

    let started = Instant::now();
    let _report = owner.publisher.wait_network_report(wait).await;

    let elapsed = started.elapsed();
    assert!(elapsed < wait + Duration::from_secs(1), "{elapsed:?}");
    owner.publisher.shutdown().await;
}
