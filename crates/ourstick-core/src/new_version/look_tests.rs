use super::super::source::{MemorySource, ReadError};
use super::super::state::MemoryNoticeStore;
use super::super::test_keys::{test_keys, version_json, TestKey};
use super::*;
use crate::clock::FixedClock;

const START: u64 = 1_800_000_000_000;

fn notice(serial: u64, newest: &str, security: &str) -> String {
    format!(
        r#"{{"serial":{serial},"made":"2027-01-15","linux":{{"newest":"{newest}","security":"{security}"}}}}"#
    )
}

struct World {
    store: MemoryNoticeStore,
    source: MemorySource,
    clock: FixedClock,
    release: TestKey,
    recovery: TestKey,
    keys: NoticeKeys,
}

impl World {
    fn new() -> Self {
        let (release, recovery, keys) = test_keys();
        Self {
            store: MemoryNoticeStore::default(),
            source: MemorySource::default(),
            clock: FixedClock::new(START),
            release,
            recovery,
            keys,
        }
    }

    async fn look(&self) -> bool {
        look_if_due(
            &self.store,
            &self.source,
            &self.clock,
            &self.keys,
            System::Linux,
        )
        .await
        .unwrap()
    }

    fn kept(&self) -> VersionLook {
        self.store.version_look().unwrap()
    }
}

#[tokio::test]
async fn a_good_notice_is_kept_and_the_next_look_is_a_day_later() {
    let world = World::new();
    world.source.answer(Ok(version_json(
        &world.release,
        &notice(1, "0.3.0", "0.2.2"),
    )));
    assert!(world.look().await);
    let kept = world.kept();
    assert_eq!(kept.newest.as_deref(), Some("0.3.0"));
    assert_eq!(kept.security.as_deref(), Some("0.2.2"));
    assert_eq!(kept.last_good_ms, Some(START));
    assert!((START + DAY_MS..START + DAY_MS + HOUR_MS).contains(&kept.next_look_ms));
    // Not due again the same day.
    world.clock.set(START + 12 * HOUR_MS);
    assert!(!world.look().await);
    assert_eq!(world.source.reads(), 1);
    // Due a day (and up to an hour) later.
    world.clock.set(START + DAY_MS + HOUR_MS);
    assert!(world.look().await);
    assert_eq!(world.source.reads(), 2);
}

#[tokio::test]
async fn a_failure_is_not_a_look_and_is_tried_again_in_an_hour() {
    let world = World::new();
    world
        .source
        .answer(Ok(b"<html>Sign in to the hotel's Wi-Fi</html>".to_vec()));
    assert!(world.look().await);
    let kept = world.kept();
    assert_eq!(kept.failure, Some(LookFailure::NotTheFile));
    assert_eq!(kept.failing_since_ms, Some(START));
    assert_eq!(kept.last_good_ms, None);
    assert_eq!(kept.next_look_ms, START + HOUR_MS);
    world.clock.set(START + HOUR_MS - 1);
    assert!(!world.look().await);
    // Still failing an hour later: since when stays the first failure.
    world.clock.set(START + HOUR_MS);
    world.source.answer(Err(ReadError::Refused(404)));
    assert!(world.look().await);
    let kept = world.kept();
    assert_eq!(kept.failure, Some(LookFailure::Refused));
    assert_eq!(kept.failing_since_ms, Some(START));
    // Each kind of failure.
    for (answer, failure) in [
        (
            Err(ReadError::NoConnection("timeout".into())),
            LookFailure::NoConnection,
        ),
        (Err(ReadError::Refused(500)), LookFailure::Refused),
        (Err(ReadError::TooBig), LookFailure::NotTheFile),
        (Ok(vec![b' '; MOST_BYTES + 1]), LookFailure::NotTheFile),
    ] {
        world.clock.set(world.kept().next_look_ms);
        world.source.answer(answer);
        assert!(world.look().await);
        assert_eq!(world.kept().failure, Some(failure));
    }
    // A good look clears it.
    world.clock.set(world.kept().next_look_ms);
    world.source.answer(Ok(version_json(
        &world.release,
        &notice(1, "0.3.0", "0.2.2"),
    )));
    assert!(world.look().await);
    assert_eq!(world.kept().failure, None);
    assert_eq!(world.kept().failing_since_ms, None);
}

#[tokio::test]
async fn a_lower_serial_is_ignored_a_higher_kept_per_key() {
    let world = World::new();
    world.source.answer(Ok(version_json(
        &world.release,
        &notice(5, "0.3.0", "0.3.0"),
    )));
    assert!(world.look().await);
    // An old notice served again cannot hide the newer one.
    world.clock.set(world.kept().next_look_ms);
    world.source.answer(Ok(version_json(
        &world.release,
        &notice(4, "0.2.0", "0.1.0"),
    )));
    assert!(world.look().await);
    assert_eq!(world.kept().newest.as_deref(), Some("0.3.0"));
    assert_eq!(world.kept().security.as_deref(), Some("0.3.0"));
    // A higher one is taken.
    world.clock.set(world.kept().next_look_ms);
    world.source.answer(Ok(version_json(
        &world.release,
        &notice(6, "0.4.0", "0.3.0"),
    )));
    assert!(world.look().await);
    assert_eq!(world.kept().newest.as_deref(), Some("0.4.0"));
    // The recovery key counts on its own.
    world.clock.set(world.kept().next_look_ms);
    world.source.answer(Ok(version_json(
        &world.recovery,
        r#"{"serial":1,"made":"2027-02-01","new_release_key":"00112233AABBCCDD"}"#,
    )));
    assert!(world.look().await);
    let kept = world.kept();
    assert_eq!(kept.new_release_key.as_deref(), Some("00112233AABBCCDD"));
    assert_eq!(kept.serials.get(&world.keys.release.id_hex()), Some(&6));
    assert_eq!(kept.serials.get(&world.keys.recovery.id_hex()), Some(&1));
}

#[tokio::test]
async fn off_nothing_is_read_and_a_newer_version_is_forgotten() {
    let world = World::new();
    world.source.answer(Ok(version_json(
        &world.release,
        &notice(1, "0.3.0", "0.3.0"),
    )));
    assert!(world.look().await);
    set_looking(&world.store, false).unwrap();
    let kept = world.kept();
    assert_eq!(kept.newest, None);
    assert_eq!(kept.security.as_deref(), Some("0.3.0"));
    world.clock.set(START + 10 * DAY_MS);
    assert!(!world.look().await);
    assert_eq!(world.source.reads(), 1);
    // On again: a look at once.
    set_looking(&world.store, true).unwrap();
    world.source.answer(Ok(version_json(
        &world.release,
        &notice(2, "0.3.0", "0.3.0"),
    )));
    assert!(world.look().await);
    assert_eq!(world.source.reads(), 2);
}

#[tokio::test]
async fn a_clock_moved_back_does_not_stop_the_looks() {
    let world = World::new();
    world.source.answer(Ok(version_json(
        &world.release,
        &notice(1, "0.3.0", "0.3.0"),
    )));
    assert!(world.look().await);
    world.clock.set(START - 30 * DAY_MS);
    world.source.answer(Ok(version_json(
        &world.release,
        &notice(2, "0.3.0", "0.3.0"),
    )));
    assert!(world.look().await);
    assert!(world.kept().next_look_ms < START);
}

#[tokio::test]
async fn told_once_is_kept() {
    let world = World::new();
    world.source.answer(Ok(version_json(
        &world.release,
        &notice(1, "0.3.0", "0.3.0"),
    )));
    world.look().await;
    told_looking(&world.store).unwrap();
    told_security(&world.store).unwrap();
    let kept = world.kept();
    assert!(kept.looking_told);
    assert_eq!(kept.security_told.as_deref(), Some("0.3.0"));
}
