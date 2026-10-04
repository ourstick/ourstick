use std::time::{Duration, Instant};

use super::*;
use crate::cipher::NoCipher;
use crate::manifest::Entry;

fn entry(number: u8, size: u64) -> Entry {
    Entry {
        path: format!("f{number:03}"),
        size,
        mtime: 0,
        hash: [number; 32],
    }
}

fn state_of(entries: &[Entry], max_bytes: u64) -> State {
    State {
        current: Some(Current {
            manifest_hash: [0xee; 32],
            plan: plan(entries, &NoCipher, max_bytes),
            entries: entries.to_vec(),
            plaintext_hashes: vec![None; entries.len()],
            layout: StickLayout::new(std::path::Path::new("/"), &[]),
        }),
        ..State::default()
    }
}

/// A version of `count` files of `size` bytes, one per suitcase.
fn state_with(count: u8, size: u64) -> (State, u64) {
    let entries: Vec<Entry> = (0..count).map(|number| entry(number, size)).collect();
    let bytes = plan::stored_len(&NoCipher, size);
    (state_of(&entries, bytes), bytes)
}

/// Takes the next job as done at once; returns the suitcases it evicted.
fn prepare_next(state: &mut State, budget: &Budget, now: Instant) -> Option<Vec<SuitcaseId>> {
    let Next::Job(job) = state.next_job(budget, now) else {
        return None;
    };
    let evicted: Vec<SuitcaseId> = job.evict.iter().map(|(id, _)| *id).collect();
    for (_, bytes) in &job.evict {
        budget.release(*bytes);
    }
    if job.beyond {
        budget.reserve_beyond(job.bytes);
    } else {
        assert!(budget.reserve(job.bytes));
    }
    let number = job.place.1;
    let ready = job.files.iter().map(|file| file.entry.hash).collect();
    state.done(Done {
        place: job.place,
        bytes: job.bytes,
        result: Ok(Some(Prepared {
            id: 100 + number as u64,
            ready,
            changed: Vec::new(),
        })),
    });
    Some(evicted)
}

#[test]
fn what_readers_wait_on_comes_first_then_the_next_one() {
    let (mut state, bytes) = state_with(5, 1_000);
    let budget = Budget::new(bytes * 10);
    let now = Instant::now();
    state.note(1, [2; 32], false, now);
    prepare_next(&mut state, &budget, now).unwrap();
    assert_eq!(state.open[0].place.1, 2);
    // Served from it: the next one is prepared ahead.
    state.note(1, [2; 32], true, now);
    assert_eq!(state.queue, [3]);
    // A reader waiting elsewhere goes before the read-ahead.
    state.note(2, [0; 32], false, now);
    assert_eq!(state.queue, [0, 3]);
    // Asked again while queued or open: not twice.
    state.note(2, [0; 32], false, now);
    state.note(1, [2; 32], false, now);
    assert_eq!(state.queue, [0, 3]);
}

#[test]
fn room_is_made_from_what_no_reader_holds() {
    let (mut state, bytes) = state_with(8, 1_000);
    let budget = Budget::new(bytes * 3);
    let now = Instant::now();
    // A reader on 0: 0 and 1 are prepared (the second ahead).
    state.note(1, [0; 32], false, now);
    prepare_next(&mut state, &budget, now).unwrap();
    state.note(1, [0; 32], true, now);
    prepare_next(&mut state, &budget, now).unwrap();
    // Another reader waits on 5.
    state.note(2, [5; 32], false, now);
    prepare_next(&mut state, &budget, now).unwrap();
    assert_eq!(budget.available(), 0);
    // The first reader moves on to 1: 0 is held by no one, and leaves for 2.
    state.note(1, [1; 32], true, now);
    assert_eq!(state.queue, [2]);
    assert_eq!(prepare_next(&mut state, &budget, now), Some(vec![100]));
    // Now 1 and 2 (reader 1) and 5 (reader 2) are held: 6 must wait.
    state.note(2, [5; 32], true, now);
    assert!(matches!(state.next_job(&budget, now), Next::NoRoom));
    // A reader that asked nothing for a while holds nothing.
    let later = now + IN_USE + Duration::from_secs(1);
    assert!(prepare_next(&mut state, &budget, later).is_some());
}

#[test]
fn a_reader_that_moved_on_does_not_hold_the_suitcase_before() {
    let (mut state, bytes) = state_with(4, 1_000);
    let budget = Budget::new(bytes);
    let now = Instant::now();
    state.note(1, [0; 32], false, now);
    prepare_next(&mut state, &budget, now).unwrap();
    // The same reader got 0 and now waits on 3: 0 may go.
    state.note(1, [0; 32], true, now);
    state.note(1, [3; 32], false, now);
    assert_eq!(prepare_next(&mut state, &budget, now), Some(vec![100]));
    // A reader that closed holds nothing either.
    state.readers.remove(&1);
    state.note(2, [1; 32], false, now);
    assert_eq!(prepare_next(&mut state, &budget, now), Some(vec![103]));
}

#[test]
fn a_file_bigger_than_the_budget_waits_until_all_else_can_go() {
    let entries = vec![entry(0, 100), entry(1, 5_000)];
    let small = plan::stored_len(&NoCipher, 100);
    let mut state = state_of(&entries, small);
    let budget = Budget::new(small * 2);
    let now = Instant::now();
    state.note(1, [0; 32], false, now);
    prepare_next(&mut state, &budget, now).unwrap();
    state.note(2, [1; 32], false, now);
    // Reader 1 is on suitcase 0: the big one waits.
    assert!(matches!(state.next_job(&budget, now), Next::NoRoom));
    state.readers.remove(&1);
    let Next::Job(job) = state.next_job(&budget, now) else {
        panic!("no job");
    };
    assert!(job.beyond);
    assert_eq!(job.evict.len(), 1);
}

#[test]
fn no_room_found_by_the_job_is_tried_again() {
    let (mut state, bytes) = state_with(2, 1_000);
    let budget = Budget::new(bytes * 4);
    let now = Instant::now();
    state.note(1, [1; 32], false, now);
    let Next::Job(job) = state.next_job(&budget, now) else {
        panic!("no job");
    };
    // The room was taken meanwhile (another stick): back in the queue.
    assert!(state.done(Done {
        place: job.place,
        bytes: job.bytes,
        result: Ok(None),
    }));
    assert_eq!(state.queue, [1]);
    assert_eq!(state.preparing, None);
}

#[test]
fn identical_files_are_served_from_whichever_suitcase_has_them() {
    let mut twin = entry(9, 100);
    twin.path = "z".to_owned();
    let entries = vec![entry(9, 100), entry(1, 100), twin];
    let one = plan::stored_len(&NoCipher, 100);
    let mut state = state_of(&entries, one);
    let budget = Budget::new(one * 10);
    let now = Instant::now();
    state.note(1, [9; 32], false, now);
    prepare_next(&mut state, &budget, now).unwrap();
    // Served from suitcase 0: the reader is placed there, not at the twin's.
    state.note(1, [9; 32], true, now);
    assert_eq!(state.readers[&1].place(now), Some((0, 0)));
}

#[test]
fn a_place_that_failed_waits_longer_each_time_and_never_spins() {
    let (mut state, bytes) = state_with(3, 1_000);
    let budget = Budget::new(bytes * 10);
    let fail = |state: &mut State, job: Job| {
        state.done(Done {
            place: job.place,
            bytes: job.bytes,
            result: Err(ShareError::SourceGone {
                name: Some("Films".to_owned()),
            }),
        })
    };
    let now = Instant::now();
    state.note(1, [1; 32], false, now);
    let Next::Job(job) = state.next_job(&budget, now) else {
        panic!("a job");
    };
    assert!(!fail(&mut state, job));
    // Asked again at once: no job, so no loop.
    state.note(1, [1; 32], false, now);
    assert!(matches!(state.next_job(&budget, now), Next::Nothing));
    // After a second, tried again; failing again, it waits two.
    let later = Instant::now() + Duration::from_millis(1_500);
    state.note(1, [1; 32], false, later);
    let Next::Job(job) = state.next_job(&budget, later) else {
        panic!("tried again");
    };
    fail(&mut state, job);
    state.note(1, [1; 32], false, later);
    assert!(matches!(state.next_job(&budget, later), Next::Nothing));
    let much_later = Instant::now() + Duration::from_millis(2_500);
    state.note(1, [1; 32], false, much_later);
    assert!(prepare_next(&mut state, &budget, much_later).is_some());
    // Prepared at last: forgotten.
    assert!(state.failed.is_empty());
    // Other places were never held back.
    state.note(2, [2; 32], false, now);
    assert!(prepare_next(&mut state, &budget, now).is_some());
}

#[test]
fn asks_across_a_suitcases_end_prepare_the_earlier_one_first() {
    let (mut state, bytes) = state_with(6, 1_000);
    let budget = Budget::new(bytes * 2);
    let now = Instant::now();
    // Several asks at once (ADR 0016) across 2's end into 3: each not
    // ready went to the front, so 3 came before 2.
    state.note(1, [2; 32], false, now);
    state.note(1, [3; 32], false, now);
    assert_eq!(state.queue, [2, 3]);
    assert_eq!(state.readers[&1].place(now), Some((0, 2)));
    let opened =
        |state: &State| -> Vec<usize> { state.open.iter().map(|open| open.place.1).collect() };
    prepare_next(&mut state, &budget, now).unwrap();
    assert_eq!(opened(&state), [2]);
    prepare_next(&mut state, &budget, now).unwrap();
    assert_eq!(opened(&state), [2, 3]);
    // 2 is still asked for while 3's blobs come: it stays held.
    state.note(1, [3; 32], true, now);
    assert!(state.is_held((0, 2), now));
    state.note(1, [5; 32], false, now);
    assert!(matches!(state.next_job(&budget, now), Next::NoRoom));
    // Served from 2 too: the reader is at 3 (never back at 2), 2 may go.
    state.note(1, [2; 32], true, now);
    assert_eq!(state.readers[&1].place(now), Some((0, 5)));
    assert!(!state.is_held((0, 2), now));
    // A later served ask never moves the place back.
    state.note(1, [5; 32], true, now);
    state.note(1, [3; 32], true, now);
    assert_eq!(state.readers[&1].place(now), Some((0, 5)));
}

#[test]
fn a_new_version_forgets_where_readers_were_in_the_old_one() {
    let (mut state, _) = state_with(4, 1_000);
    let now = Instant::now();
    state.note(1, [2; 32], false, now);
    // A new version (the same files): the old waiting ask can never be
    // served, and must not keep the reader in the old version.
    state.next_generation();
    state.note(1, [2; 32], false, now);
    assert_eq!(state.readers[&1].place(now), Some((1, 2)));
    // A place asked for long ago is no place.
    let later = now + IN_USE + Duration::from_secs(1);
    assert_eq!(state.readers[&1].place(later), None);
}

#[test]
fn a_suitcase_s_files_are_placed_from_the_layout_when_it_is_prepared() {
    let mut state = state_of(&[entry(1, 10), entry(2, 10)], 1 << 20);
    let current = state.current.as_mut().unwrap();
    current.layout = StickLayout::new(std::path::Path::new("/stick"), &[]);
    current.plaintext_hashes[1] = Some([9; 32]);
    let jobs = current.jobs(0..2);
    assert_eq!(jobs[0].on_disk, std::path::Path::new("/stick/f001"));
    assert_eq!(jobs[1].entry, entry(2, 10));
    assert_eq!(
        (jobs[0].plaintext_hash, jobs[1].plaintext_hash),
        (None, Some([9; 32]))
    );
    assert_eq!(jobs[1].root.path, std::path::Path::new("/stick"));
}
