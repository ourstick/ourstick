//! The judgement with the kernel's counters (ADR 0013, amendment 1).

use std::time::Duration;

use super::*;
use crate::disk::counters::Counters;

const MB: u64 = 1 << 20;

fn ms(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

/// A window of 64 MB read in `took`, the disk moving `disk_read` and
/// `disk_written` bytes in `ops` operations meanwhile; OurStick made 1024
/// read calls.
fn window(
    share: &mut DiskShare,
    took: u64,
    disk_read: u64,
    disk_written: u64,
    ops: u64,
) -> Duration {
    share.after_window_seen(
        ReaderKind::Transfers,
        0,
        64 * MB,
        ms(took),
        Seen {
            disk: Counters {
                ops,
                read_bytes: disk_read,
                written_bytes: disk_written,
            },
            ours: 64 * MB,
            reads: 1024,
        },
    )
}

/// Three windows at the disk's speed, alone.
fn learned() -> DiskShare {
    let mut share = DiskShare::default();
    for _ in 0..3 {
        window(&mut share, 500, 64 * MB, 0, 512);
    }
    share
}

#[test]
fn slow_windows_with_no_one_else_do_not_give_way() {
    let mut share = learned();
    // Ten times slower (photos after films, files no longer in memory),
    // and the disk did nothing but OurStick's reads.
    for _ in 0..4 {
        assert_eq!(window(&mut share, 5000, 64 * MB, 0, 900), Duration::ZERO);
    }
    assert!(!share.giving_way());
}

#[test]
fn someone_moving_as_much_as_ourstick_is_seen_whatever_the_time() {
    let mut share = learned();
    // At the usual speed, while a copy reads 64 MB and writes 64 MB.
    window(&mut share, 500, 128 * MB, 64 * MB, 600);
    assert!(!share.giving_way(), "one window alone is never enough");
    assert!(window(&mut share, 500, 128 * MB, 64 * MB, 600) > Duration::ZERO);
    assert!(share.giving_way());
    // The copy ends: full speed at the next window.
    assert_eq!(window(&mut share, 500, 64 * MB, 0, 512), Duration::ZERO);
    assert!(!share.giving_way());
}

#[test]
fn a_slow_disk_busy_with_small_things_is_someone_else() {
    let mut share = learned();
    // Few bytes but many operations (a database here and there), and slow.
    for _ in 0..2 {
        window(&mut share, 2000, 64 * MB + MB, 0, 1024 + 300);
    }
    assert!(share.giving_way());
}

#[test]
fn read_ahead_and_metadata_are_not_someone_else() {
    let mut share = learned();
    // A little more than OurStick read (read ahead), at its usual speed.
    for _ in 0..4 {
        window(&mut share, 600, 64 * MB + 3 * MB, 0, 600);
    }
    assert!(!share.giving_way());
}

#[test]
fn a_window_mostly_from_memory_is_not_judged() {
    let mut share = learned();
    // OurStick read 64 MB, 1 MB of it from the disk; others moved 50 MB.
    for _ in 0..3 {
        window(&mut share, 50, MB, 50 * MB, 100);
    }
    assert!(!share.giving_way());
}

#[test]
fn without_counters_timing_decides_as_before() {
    let mut share = DiskShare::default();
    for _ in 0..3 {
        DiskLoad::after_window(&mut share, ReaderKind::Transfers, 0, 64 * MB, ms(500), None);
    }
    DiskLoad::after_window(
        &mut share,
        ReaderKind::Transfers,
        0,
        64 * MB,
        ms(5000),
        None,
    );
    DiskLoad::after_window(
        &mut share,
        ReaderKind::Transfers,
        0,
        64 * MB,
        ms(5000),
        None,
    );
    assert!(share.giving_way());
}
