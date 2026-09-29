//! The owner sees readers: how many are downloading and how fast data leaves,
//! and that nobody is once they are done.

mod common;

use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use common::{noise, Owner, Reader};
use ourstick_core::transport::{Fetcher, PeerConnection, Publisher, ReaderActivity};

#[tokio::test(flavor = "multi_thread")]
async fn a_download_shows_as_one_reader_with_a_rate_then_none() {
    let data = noise(4_000_000, 7);
    let owner = Owner::new(&[("big.bin", &data)]).await;
    let publication = owner.publish().await;
    let ticket = owner.ticket(&publication);
    let hash = publication.manifest.entries[0].hash;
    let mut activity = owner.publisher.activity();
    assert_eq!(*activity.borrow(), ReaderActivity::default());

    let reader = Reader::new();
    let fetcher = reader.fetcher().await;
    let download = async {
        let connection = fetcher.connect(&ticket).await.unwrap();
        // Slowly, so the owner has time to see it.
        connection
            .fetch_blob(&hash, &mut |_| {
                ControlFlow::Continue(Duration::from_millis(5))
            })
            .await
            .unwrap();
        connection.close().await;
    };
    let watch = async {
        let mut busiest = ReaderActivity::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if activity.changed().await.is_err() {
                break;
            }
            let now = *activity.borrow();
            busiest.readers = busiest.readers.max(now.readers);
            busiest.bytes_per_second = busiest.bytes_per_second.max(now.bytes_per_second);
            if busiest.readers > 0 && now.readers == 0 {
                break;
            }
        }
        busiest
    };
    let ((), busiest) = tokio::join!(download, watch);

    assert_eq!(busiest.readers, 1, "{busiest:?}");
    assert!(busiest.bytes_per_second > 0, "{busiest:?}");
    assert_eq!(activity.borrow().readers, 0, "nobody downloads any more");
    fetcher.shutdown(false).await.unwrap();
    owner.publisher.shutdown().await;
}
