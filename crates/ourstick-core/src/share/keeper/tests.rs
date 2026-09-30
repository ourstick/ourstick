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
    let files = entries
        .iter()
        .map(|entry| FileJob {
            entry: entry.clone(),
            on_disk: PathBuf::from(&entry.path),
            plaintext_hash: None,
        })
        .collect();
    State {
        current: Some(Current {
            manifest_hash: [0xee; 32],
            plan: plan(entries, &NoCipher, max_bytes),
            files,
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
fn a_reader_waiting_on_one_suitcase_does_not_hold_the_one_before() {
    let (mut state, bytes) = state_with(4, 1_000);
    let budget = Budget::new(bytes);
    let now = Instant::now();
    state.note(1, [0; 32], false, now);
    prepare_next(&mut state, &budget, now).unwrap();
    // The same reader now waits on 3: 0 may go.
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
    assert_eq!(state.readers[&1].0, (0, 0));
}
