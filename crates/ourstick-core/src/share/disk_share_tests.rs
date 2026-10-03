use super::*;

const MB: u64 = 1 << 20;

fn ms(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

/// A window of 64 MB of big files at `mb_per_second`.
fn big(share: &mut DiskShare, mb_per_second: u64) -> Duration {
    share.after_window(2, 64 * MB, ms(64_000 / mb_per_second))
}

/// A window of 64 small files at `milliseconds` each.
fn small(share: &mut DiskShare, milliseconds: u64) -> Duration {
    share.after_window(64, 64 * 100_000, ms(64 * milliseconds))
}

#[test]
fn a_steady_disk_never_gives_way() {
    let mut share = DiskShare::default();
    for _ in 0..1000 {
        assert_eq!(big(&mut share, 120), Duration::ZERO);
        // Up to three times slower now and then (inner tracks, a fragment).
        assert_eq!(big(&mut share, 50), Duration::ZERO);
        assert_eq!(small(&mut share, 10), Duration::ZERO);
        assert_eq!(small(&mut share, 25), Duration::ZERO);
    }
    assert!(!share.giving_way());
}

#[test]
fn a_busy_stretch_gives_way_after_two_slow_windows_and_back_after_one_fast() {
    let mut share = DiskShare::default();
    for _ in 0..5 {
        big(&mut share, 120);
    }
    // One slow window alone: nothing changes.
    assert_eq!(big(&mut share, 20), Duration::ZERO);
    assert_eq!(big(&mut share, 120), Duration::ZERO);
    assert_eq!(big(&mut share, 20), Duration::ZERO);
    // A second in a row: OurStick gives way, nine times the window.
    let pause = big(&mut share, 20);
    assert!(share.giving_way());
    assert_eq!(pause, ms(64_000 / 20) * 9);
    // Still slow: still giving way.
    assert!(big(&mut share, 25) > Duration::ZERO);
    // In between (twice the best): neither, it goes on giving way.
    assert!(big(&mut share, 60) > Duration::ZERO);
    // Fast again: full speed at once.
    assert_eq!(big(&mut share, 110), Duration::ZERO);
    assert!(!share.giving_way());
}

#[test]
fn small_and_big_files_are_judged_each_by_their_own_kind() {
    let mut share = DiskShare::default();
    // A small file a seek (10 ms) is not slow beside big files' bytes, nor a
    // big file's bytes beside small files.
    for _ in 0..10 {
        big(&mut share, 120);
        small(&mut share, 10);
    }
    assert!(!share.giving_way());
    // But small files getting far slower are.
    small(&mut share, 40);
    small(&mut share, 40);
    assert!(share.giving_way());
}

#[test]
fn a_window_of_big_files_or_inside_one_counts_bytes() {
    let mut share = DiskShare::default();
    // Inside one big file, no file begins.
    share.after_window(0, 64 * MB, ms(500));
    share.after_window(0, 64 * MB, ms(500));
    share.after_window(0, 64 * MB, ms(500));
    share.after_window(0, 64 * MB, ms(2000));
    assert!(!share.giving_way());
    share.after_window(0, 64 * MB, ms(2000));
    assert!(share.giving_way());
}

#[test]
fn what_is_read_from_memory_changes_nothing_and_teaches_nothing() {
    let mut share = DiskShare::default();
    // Files just copied in: 64 MB in 20 ms, small files in 50 µs.
    share.after_window(2, 64 * MB, ms(20));
    share.after_window(64, 64 * 1000, Duration::from_micros(64 * 50));
    // The disk's own speed afterwards is not taken for someone else…
    for _ in 0..10 {
        assert_eq!(big(&mut share, 100), Duration::ZERO);
        assert_eq!(small(&mut share, 10), Duration::ZERO);
    }
    assert!(!share.giving_way());
    // …and while giving way, a window from memory (the end of a part, read
    // ahead by the system) does not undo it, nor break two slow in a row.
    big(&mut share, 20);
    share.after_window(0, 3 * MB, ms(0));
    big(&mut share, 20);
    assert!(share.giving_way());
    assert!(share.after_window(0, 3 * MB, ms(0)) >= Duration::from_secs(1));
    assert!(share.giving_way());
    // Only the disk at its speed again ends it.
    big(&mut share, 100);
    assert!(!share.giving_way());
}

#[test]
fn a_slower_stretch_of_the_disk_stops_counting_as_someone_else() {
    let mut share = DiskShare::default();
    big(&mut share, 150);
    big(&mut share, 150);
    big(&mut share, 150);
    // Four times slower for good (another, slower part of the disk): it
    // gives way at first…
    big(&mut share, 35);
    big(&mut share, 35);
    assert!(share.giving_way());
    // …then, as the best fades with reading time (each window here is
    // nearly 2 s of it; paused or not, only reading counts), the slower
    // speed is no longer slow, and OurStick goes back to full speed.
    let mut windows = 0;
    while share.giving_way() {
        big(&mut share, 35);
        windows += 1;
        assert!(windows < 10_000, "never back to full speed");
    }
    assert!(windows > 100, "back too soon: {windows} windows");
}

#[test]
fn the_pause_is_nine_times_the_window_within_bounds() {
    let mut share = DiskShare::default();
    small(&mut share, 1);
    small(&mut share, 1);
    small(&mut share, 1);
    // Slow small windows of 64 × 4 ms = 256 ms: nine times is 2.3 s.
    small(&mut share, 4);
    assert_eq!(small(&mut share, 4), ms(64 * 4) * 9);
    // A very short window still pauses a second…
    let mut short = DiskShare::default();
    short.after_window(64, 64 * 1000, ms(64));
    short.after_window(64, 64 * 1000, ms(64));
    short.after_window(64, 64 * 1000, ms(64));
    short.after_window(64, 64 * 1000, ms(64 * 5));
    assert_eq!(short.after_window(64, 64 * 1000, ms(64 * 5)), ms(2880));
    let mut tiny = DiskShare::default();
    tiny.after_window(1, 1000, ms(1));
    tiny.after_window(1, 1000, ms(1));
    tiny.after_window(1, 1000, ms(1));
    tiny.after_window(1, 1000, ms(10));
    assert_eq!(tiny.after_window(1, 1000, ms(10)), SHORTEST_PAUSE);
    // …and a very long one at most 30 s.
    let mut long = DiskShare::default();
    big(&mut long, 120);
    big(&mut long, 120);
    big(&mut long, 120);
    big(&mut long, 5);
    assert_eq!(big(&mut long, 5), LONGEST_PAUSE);
}

#[test]
fn an_empty_window_changes_nothing() {
    let mut share = DiskShare::default();
    assert_eq!(share.after_window(0, 0, ms(5)), Duration::ZERO);
    big(&mut share, 120);
    big(&mut share, 120);
    big(&mut share, 120);
    big(&mut share, 20);
    big(&mut share, 20);
    assert!(share.giving_way());
    assert!(share.after_window(0, 0, ms(5)) >= SHORTEST_PAUSE);
    assert!(share.giving_way());
}

#[test]
fn one_window_read_partly_from_memory_sets_no_speed_to_keep() {
    let mut share = DiskShare::default();
    for _ in 0..5 {
        big(&mut share, 120);
    }
    // Half of a window was still in memory: 600 MB/s, under the floor of
    // what only memory does, so it is taken as the disk's.
    big(&mut share, 600);
    // The disk's own speed after it is not slow: what a window should take
    // is the second best, still the disk's.
    for _ in 0..10 {
        assert_eq!(big(&mut share, 110), Duration::ZERO);
    }
    assert!(!share.giving_way());
}

#[test]
fn disk_share_is_a_disk_load() {
    let mut load: Box<dyn DiskLoad> = Box::new(DiskShare::default());
    for _ in 0..3 {
        load.after_window(2, 64 * MB, ms(500));
    }
    load.after_window(2, 64 * MB, ms(3000));
    assert!(load.after_window(2, 64 * MB, ms(3000)) > Duration::ZERO);
    assert!(load.giving_way());
    assert_eq!(load.pause_for(ms(500)), ms(4500));
}

#[test]
fn someone_from_the_fourth_window_on_is_seen_too() {
    let mut share = DiskShare::default();
    // Three windows alone at the disk's speed (none judged yet), then
    // someone else at once: slow windows are not learned, so they are seen.
    for _ in 0..3 {
        big(&mut share, 120);
    }
    big(&mut share, 30);
    big(&mut share, 30);
    assert!(share.giving_way(), "a slow window became the measure");
}

#[test]
fn someone_there_from_the_start_is_taken_for_the_disk_until_it_is_alone() {
    let mut share = DiskShare::default();
    // Someone else from the first window: their speed is the measure…
    for _ in 0..20 {
        assert_eq!(big(&mut share, 30), Duration::ZERO);
    }
    // …until the disk is alone, which is learned as it comes.
    for _ in 0..20 {
        big(&mut share, 120);
    }
    big(&mut share, 30);
    big(&mut share, 30);
    assert!(share.giving_way());
}

#[test]
fn windows_read_ahead_by_the_system_are_no_measure() {
    // As measured on a spinning disk: the system reads ahead while OurStick
    // encrypts, so some windows look far faster (365 MB/s) than the disk
    // (100 MB/s). Alone on the disk, OurStick never gives way.
    let mut share = DiskShare::default();
    for round in 0..200 {
        let mb_per_second = if round % 3 == 0 { 365 } else { 100 };
        assert_eq!(
            big(&mut share, mb_per_second),
            Duration::ZERO,
            "round {round}"
        );
    }
    // Someone else, at a fifth of the disk's speed: it gives way.
    big(&mut share, 30);
    big(&mut share, 30);
    assert!(share.giving_way());
}
