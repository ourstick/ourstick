//! A stick's files are downloaded in batches (ADR 0007): runs of entries in
//! manifest order, each fetched into a store of its own that is deleted once
//! every entry of the batch is in place. The reader then needs about the size
//! of what it downloads, plus one batch, instead of twice the stick.
//!
//! Batches are cut from the whole manifest, not from what is still missing, so
//! that a second run of the same version finds the same batches and resumes the
//! partial blob of the one it stopped in.

use std::ops::Range;

use crate::manifest::Entry;

/// A batch closes before it holds more than this many bytes (unless one file
/// alone is bigger: that file is a batch of its own).
pub(super) const BATCH_BYTES: u64 = 1 << 30;

/// What a download needs on the data folder's disk while it runs: its
/// batches, this one and the next (for the space ledger).
pub const SCRATCH_BYTES: u64 = 2 * BATCH_BYTES;

/// A batch closes at this many files, so batches of tiny files stay small in
/// number of blobs too.
pub(super) const BATCH_FILES: usize = 512;

/// One batch: a run of manifest entries and the name of its store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Batch {
    pub(super) name: String,
    pub(super) entries: Range<usize>,
}

/// Cuts `entries` into batches of at most `max_bytes` and `max_files`.
pub(super) fn plan_batches(entries: &[Entry], max_bytes: u64, max_files: usize) -> Vec<Batch> {
    let mut batches = Vec::new();
    let mut start = 0;
    let mut bytes = 0u64;
    for (index, entry) in entries.iter().enumerate() {
        let count = index - start;
        let full = count >= max_files || (count > 0 && bytes + entry.size > max_bytes);
        if full {
            batches.push(batch(entries, batches.len(), start..index));
            start = index;
            bytes = 0;
        }
        bytes += entry.size;
    }
    if start < entries.len() {
        batches.push(batch(entries, batches.len(), start..entries.len()));
    }
    batches
}

/// Named by its place and its first blob: a changed manifest that moves the
/// batch's start gives it another name, so no store is reused for other
/// files.
fn batch(entries: &[Entry], number: usize, range: Range<usize>) -> Batch {
    let first = data_encoding::HEXLOWER.encode(&entries[range.start].hash[..8]);
    Batch {
        name: format!("b{number}-{first}"),
        entries: range,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::is_batch_name;

    fn entry(size: u64, fill: u8) -> Entry {
        Entry {
            path: format!("f{fill}"),
            size,
            mtime: 0,
            hash: [fill; 32],
        }
    }

    #[test]
    fn batches_close_on_bytes_or_files() {
        let entries: Vec<_> = [40, 40, 30, 5, 5, 5, 5, 200, 1]
            .iter()
            .enumerate()
            .map(|(index, size)| entry(*size, index as u8))
            .collect();
        let batches = plan_batches(&entries, 100, 3);
        let ranges: Vec<_> = batches.iter().map(|batch| batch.entries.clone()).collect();
        // 40+40 then 30 would pass 100; 30,5,5 is 3 files; 5,5 then 200 would
        // pass; 200 alone is bigger than a batch; then the rest.
        assert_eq!(ranges, vec![0..2, 2..5, 5..7, 7..8, 8..9]);
        assert!(batches.iter().all(|batch| is_batch_name(&batch.name)));
        assert_eq!(batches[1].name, "b1-0202020202020202");
    }

    #[test]
    fn the_same_manifest_gives_the_same_batches() {
        let entries: Vec<_> = (0..10).map(|fill| entry(10, fill)).collect();
        assert_eq!(
            plan_batches(&entries, 25, 100),
            plan_batches(&entries, 25, 100)
        );
        assert!(plan_batches(&[], 25, 100).is_empty());
    }
}
