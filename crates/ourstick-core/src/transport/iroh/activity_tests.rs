use super::*;

#[test]
fn readers_are_connections_with_a_transfer_in_progress() {
    let activity = Activity::new();
    activity.started(1);
    activity.started(1);
    activity.started(2);
    assert_eq!(activity.counters().active.len(), 2);
    activity.ended(1);
    activity.ended(2);
    assert_eq!(activity.counters().active.len(), 1);
    activity.ended(1);
    activity.ended(1); // an extra end does no harm
    assert!(activity.counters().active.is_empty());
}

#[test]
fn a_reader_between_two_files_still_counts() {
    let start = Instant::now();
    let activity = Activity::new();
    activity.started(1);
    activity.ended_at(1, start);
    let readers = |at: Duration| activity.counters().readers(start + at);
    assert_eq!(readers(Duration::from_secs(5)), 1);
    // The next file: still one reader.
    activity.started(1);
    assert_eq!(readers(Duration::from_secs(6)), 1);
    activity.ended_at(1, start + Duration::from_secs(10));
    assert_eq!(readers(LINGER), 1);
    // Idle for longer than LINGER: gone.
    assert_eq!(readers(LINGER + Duration::from_secs(10)), 0);
    assert!(activity.counters().idle_since.is_empty());
}

#[test]
fn a_closed_connection_stops_counting_at_once() {
    let start = Instant::now();
    let activity = Activity::new();
    activity.started(1);
    activity.started(2);
    activity.ended_at(1, start);
    activity.closed(1);
    activity.closed(2);
    assert_eq!(activity.counters().readers(start), 0);
}

#[test]
fn a_reader_told_not_ready_is_waiting_for_files_got_ready() {
    let start = Instant::now();
    let activity = Activity::new();
    let at = |seconds| start + Duration::from_secs(seconds);
    assert!(!activity.counters().preparing(at(0)));
    // Told "not ready": a reader, waiting while its files are got ready.
    activity.told_not_ready_at(7, at(1));
    assert_eq!(activity.counters().readers(at(2)), 1);
    assert!(activity.counters().preparing(at(2)));
    // Still a reader while it waits, past LINGER, up to its longest
    // pause (it asks again within a minute).
    assert_eq!(
        activity
            .counters()
            .readers(at(1) + LINGER + Duration::from_secs(20)),
        1
    );
    // Bytes leave: no longer only waiting.
    activity.sent_at(1000, at(3));
    assert!(!activity.counters().preparing(at(4)));
    // Told again later, and waiting as long as a reader's longest pause.
    activity.told_not_ready_at(7, at(5));
    assert!(activity
        .counters()
        .preparing(at(5) + PREPARING - Duration::from_secs(1)));
    assert!(!activity.counters().preparing(at(5) + PREPARING));
    // A transfer in progress on that connection is not made idle, and
    // counts once.
    activity.started(8);
    activity.told_not_ready_at(8, at(6));
    assert_eq!(activity.counters().active.get(&8), Some(&1));
    // A reader that leaves stops counting at once.
    activity.closed(7);
    activity.closed(8);
    assert_eq!(activity.counters().readers(at(7)), 0);
}

#[test]
fn only_relayed_until_someone_comes_directly() {
    let only_relayed = |activity: &Activity| activity.counters().only_relayed();
    let activity = Activity::new();
    assert!(!only_relayed(&activity), "no reader yet");
    activity.reached_only_by_relay();
    assert!(only_relayed(&activity));
    activity.reached_directly();
    assert!(!only_relayed(&activity), "one direct reader is enough");
    activity.reached_only_by_relay();
    assert!(!only_relayed(&activity));
}

#[test]
fn a_declined_local_network_counts_while_its_connection_lasts() {
    let activity = Activity::new();
    let declined = |activity: &Activity| !activity.counters().declined.is_empty();
    activity.local_network_declined(1, true);
    activity.local_network_declined(2, true);
    assert!(declined(&activity));
    activity.local_network_declined(1, false);
    assert!(declined(&activity), "still for the other");
    activity.closed(2);
    assert!(!declined(&activity), "a closed connection no longer counts");
}

#[tokio::test(start_paused = true)]
async fn the_ticker_sleeps_while_no_one_downloads() {
    let activity = Activity::new();
    let mut seen = activity.subscribe();
    let reporting = tokio::spawn(activity.clone().report(Arc::new(|| false)));
    // Long quiet: nothing changes, nothing is sent.
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert!(!seen.has_changed().unwrap());
    // A reader starts: seen within a tick.
    activity.started(1);
    tokio::time::sleep(TICK * 2).await;
    assert!(seen.has_changed().unwrap());
    assert_eq!(seen.borrow_and_update().readers, 1);
    reporting.abort();
}

#[test]
fn rate_is_over_the_samples_kept() {
    let start = Instant::now();
    let samples: VecDeque<_> = [
        (start, 0),
        (start + Duration::from_millis(500), 1_000_000),
        (start + Duration::from_secs(2), 4_000_000),
    ]
    .into();
    assert_eq!(rate(&samples), 2_000_000);
    assert_eq!(rate(&VecDeque::new()), 0);
    assert_eq!(rate(&[(start, 5)].into()), 0);
}
