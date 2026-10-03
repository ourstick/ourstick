//! The files being placed at once (ADR 0014): small files are decrypted,
//! synced and moved into place several together, so their syncs wait
//! together and the filesystem joins them; a big file is placed alone.
//! They are finished in the order they came.

use std::collections::VecDeque;

/// At most this many files are placed at once: on the measured disks, a
/// sync of a small file cost 4.6 ms alone and 0.56 ms among 16 (NVMe), and
/// 36 ms against 10 ms (a spinning disk).
pub(super) const PLACING_AT_ONCE: usize = 16;
/// Only files of at most this size join others; a bigger one is placed
/// alone (its one sync is nothing beside its bytes).
pub(super) const QUEUED_FILE_BYTES: u64 = 4 << 20;
/// At most this many bytes are placed at once, so a stop waits for what is
/// in flight about a second (64 MB, decrypted and written at a few hundred
/// MB/s), not for many big files.
pub(super) const QUEUED_BYTES: u64 = 64 << 20;

/// What is in flight, oldest first. `T` is the placement (a task), with
/// its blob's hash and its size.
#[derive(Debug)]
pub(super) struct PlacingQueue<T> {
    placing: VecDeque<(T, [u8; 32], u64)>,
    bytes: u64,
}

impl<T> Default for PlacingQueue<T> {
    fn default() -> Self {
        Self {
            placing: VecDeque::new(),
            bytes: 0,
        }
    }
}

impl<T> PlacingQueue<T> {
    pub(super) fn is_empty(&self) -> bool {
        self.placing.is_empty()
    }

    /// Whether a file of `size` bytes can start now, beside what is in
    /// flight. When not, the oldest is finished first.
    pub(super) fn has_room_for(&self, size: u64) -> bool {
        if self.placing.is_empty() {
            return true;
        }
        let alone = |size: u64| size > QUEUED_FILE_BYTES;
        !alone(size)
            && !self.placing.iter().any(|(_, _, size)| alone(*size))
            && self.placing.len() < PLACING_AT_ONCE
            && self.bytes + size <= QUEUED_BYTES
    }

    /// Whether a file reading `hash`'s blob is in flight: a twin waits for
    /// it (its file in `partial` is named after the blob).
    pub(super) fn holds_blob(&self, hash: &[u8; 32]) -> bool {
        self.placing.iter().any(|(_, held, _)| held == hash)
    }

    pub(super) fn push(&mut self, placing: T, hash: [u8; 32], size: u64) {
        self.bytes += size;
        self.placing.push_back((placing, hash, size));
    }

    /// The oldest in flight, to finish.
    pub(super) fn pop(&mut self) -> Option<T> {
        let (placing, _, size) = self.placing.pop_front()?;
        self.bytes -= size;
        Some(placing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: u64 = 4096;

    #[test]
    fn small_files_go_together_up_to_the_count_and_the_bytes() {
        let mut queue = PlacingQueue::default();
        assert!(queue.is_empty() && queue.has_room_for(SMALL));
        for index in 0..PLACING_AT_ONCE {
            assert!(queue.has_room_for(SMALL));
            queue.push(index, [index as u8; 32], SMALL);
        }
        assert!(!queue.has_room_for(SMALL));
        // Oldest first.
        assert_eq!(queue.pop(), Some(0));
        assert!(queue.has_room_for(SMALL));
        // Bytes: 16 files of 4 MB are 64 MB, the most in flight.
        let mut queue = PlacingQueue::default();
        for index in 0..PLACING_AT_ONCE {
            queue.push(index, [0; 32], QUEUED_FILE_BYTES);
        }
        assert_eq!(queue.bytes, QUEUED_BYTES);
        queue.pop();
        assert!(queue.has_room_for(QUEUED_FILE_BYTES));
        assert!(!queue.has_room_for(QUEUED_FILE_BYTES + 1));
    }

    #[test]
    fn a_big_file_is_placed_alone() {
        let big = QUEUED_FILE_BYTES + 1;
        let mut queue = PlacingQueue::default();
        // Alone: it starts only when nothing else is in flight…
        assert!(queue.has_room_for(big));
        queue.push("small", [1; 32], SMALL);
        assert!(!queue.has_room_for(big));
        assert_eq!(queue.pop(), Some("small"));
        queue.push("big", [2; 32], big);
        // …and nothing starts beside it.
        assert!(!queue.has_room_for(SMALL));
        assert_eq!(queue.pop(), Some("big"));
        assert!(queue.is_empty() && queue.has_room_for(SMALL));
    }

    #[test]
    fn a_twin_finds_its_blob_in_flight_until_it_is_placed() {
        let mut queue = PlacingQueue::default();
        queue.push(1, [7; 32], SMALL);
        queue.push(2, [8; 32], SMALL);
        queue.push(3, [7; 32], SMALL);
        assert!(queue.holds_blob(&[7; 32]) && !queue.holds_blob(&[9; 32]));
        queue.pop();
        queue.pop();
        // The last with that blob is still in flight.
        assert!(queue.holds_blob(&[7; 32]));
        queue.pop();
        assert!(!queue.holds_blob(&[7; 32]));
        assert_eq!(queue.pop(), None);
    }
}
