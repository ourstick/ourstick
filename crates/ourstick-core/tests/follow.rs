//! `get --follow` waits for the owner's next version, and rides out a time
//! when the owner cannot be reached.

mod common;

use std::time::Duration;

use common::memory::{publish_versions, Link, MemoryFetcher};
use ourstick_core::download::{wait_for_newer, FollowEvent};

const EVERY: Duration = Duration::from_millis(50);

fn fetcher(stick: &common::memory::Versions) -> MemoryFetcher {
    MemoryFetcher::new(
        stick.served.clone(),
        Link {
            direct: true,
            ..Link::default()
        },
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn returns_when_the_owner_has_a_newer_version() {
    let stick = publish_versions(&[&[("a", b"one")], &[("a", b"two")]]).await;
    let fetcher = fetcher(&stick);
    let mut events = Vec::new();

    let mut on_event = |event: FollowEvent<'_>| events.push(format!("{event:?}"));
    let (seq, ()) = tokio::join!(
        wait_for_newer(&fetcher, &stick.ticket, 1_000, EVERY, &mut on_event),
        async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            stick.publish(1);
        }
    );

    assert_eq!(seq.unwrap(), 2_000);
    assert!(events.is_empty(), "{events:?}");
    // A connection is kept between questions, not opened every time.
    assert_eq!(*fetcher.connects.lock().unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_owner_is_reported_once_and_waited_for() {
    let stick = publish_versions(&[&[("a", b"one")], &[("a", b"two")]]).await;
    let fetcher = fetcher(&stick);
    let mut events = Vec::new();
    stick.withdraw();

    let mut on_event = |event: FollowEvent<'_>| {
        events.push(match event {
            FollowEvent::Unreachable { .. } => "unreachable",
            FollowEvent::Reachable => "reachable",
        })
    };
    let (seq, ()) = tokio::join!(
        wait_for_newer(&fetcher, &stick.ticket, 1_000, EVERY, &mut on_event),
        async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            stick.publish(0);
            tokio::time::sleep(Duration::from_millis(200)).await;
            stick.publish(1);
        }
    );

    assert_eq!(seq.unwrap(), 2_000);
    assert_eq!(events, vec!["unreachable", "reachable"]);
}
