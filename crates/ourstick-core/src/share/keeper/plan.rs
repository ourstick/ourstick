//! Which files go in which suitcase: runs of manifest entries in manifest
//! order, each up to a size on disk (ADR 0007). A reader goes through the
//! manifest in order, so it goes through suitcases one after the other.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::blob::blob_len;
use crate::cipher::Cipher;
use crate::manifest::Entry;
use crate::share::outboard_len;
use crate::transport::BlobHash;

/// A suitcase closes at this size, or at a sixth of the budget if that is
/// less, so a reader never waits long for the first one and two readers each
/// have room for the one they are on and the next, with room to spare.
pub(super) const SUITCASE_BYTES: u64 = 1 << 30;

/// A suitcase also closes at this many blobs. Each suitcase is a store whose
/// database keeps every page it writes in memory (redb's cache, up to
/// 1 GiB, which iroh-blobs does not let us set): a suitcase of 100 000
/// small files grew the sender by hundreds of megabytes. Closed, its
/// memory goes with it.
pub(super) const SUITCASE_FILES: usize = 4096;

/// The suitcases of one version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Plan {
    pub(super) suitcases: Vec<Planned>,
    /// Which suitcases hold a blob (identical files may be in several).
    pub(super) by_hash: HashMap<BlobHash, Vec<usize>>,
}

/// One suitcase to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Planned {
    pub(super) entries: Range<usize>,
    /// Its size on disk once prepared: blobs and outboards, each blob once.
    pub(super) bytes: u64,
}

/// The size on disk of the blob of a file of `size` bytes, outboard included.
pub(super) fn stored_len(cipher: &dyn Cipher, size: u64) -> u64 {
    let blob = blob_len(cipher, size);
    blob + outboard_len(blob)
}

/// Cuts `entries` into suitcases of at most `max_bytes` on disk (a file
/// bigger than that is a suitcase of its own) and [`SUITCASE_FILES`] blobs.
pub(super) fn plan(entries: &[Entry], cipher: &dyn Cipher, max_bytes: u64) -> Plan {
    let mut suitcases: Vec<Planned> = Vec::new();
    let mut by_hash: HashMap<BlobHash, Vec<usize>> = HashMap::new();
    let mut start = 0;
    let mut bytes = 0u64;
    let mut held: HashSet<BlobHash> = HashSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let len = stored_len(cipher, entry.size);
        let new_blob = !held.contains(&entry.hash);
        let full = bytes + len > max_bytes || (new_blob && held.len() >= SUITCASE_FILES);
        if index > start && full {
            suitcases.push(Planned {
                entries: start..index,
                bytes,
            });
            start = index;
            bytes = 0;
            held.clear();
        }
        let number = suitcases.len();
        let places = by_hash.entry(entry.hash).or_default();
        // Identical files share a blob: once per suitcase.
        if held.insert(entry.hash) {
            bytes += len;
            places.push(number);
        }
    }
    if start < entries.len() {
        suitcases.push(Planned {
            entries: start..entries.len(),
            bytes,
        });
    }
    Plan { suitcases, by_hash }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::NoCipher;

    fn entry(size: u64, fill: u8) -> Entry {
        Entry {
            path: format!("f{fill:03}"),
            size,
            mtime: 0,
            hash: [fill; 32],
        }
    }

    #[test]
    fn suitcases_are_runs_in_manifest_order_up_to_a_size() {
        let cipher = NoCipher;
        let entries = vec![
            entry(400, 1),
            entry(400, 2),
            entry(400, 3),
            entry(5_000, 4),
            entry(10, 5),
        ];
        let max = stored_len(&cipher, 400) * 2;
        let plan = plan(&entries, &cipher, max);
        let runs: Vec<_> = plan.suitcases.iter().map(|s| s.entries.clone()).collect();
        // Two fit; the big one alone; then the rest.
        assert_eq!(runs, vec![0..2, 2..3, 3..4, 4..5]);
        assert_eq!(plan.suitcases[0].bytes, max);
        assert_eq!(plan.by_hash[&[4; 32]], vec![2]);
    }

    #[test]
    fn identical_files_count_once_per_suitcase_and_are_found_in_each() {
        let cipher = NoCipher;
        let mut twin = entry(100, 9);
        twin.path = "g".to_owned();
        let entries = vec![entry(100, 9), twin.clone(), entry(100, 1), twin];
        let len = stored_len(&cipher, 100);
        let plan = plan(&entries, &cipher, len * 2);
        let runs: Vec<_> = plan.suitcases.iter().map(|s| s.entries.clone()).collect();
        assert_eq!(runs, vec![0..3, 3..4]);
        assert_eq!(plan.suitcases[0].bytes, len * 2);
        assert_eq!(plan.by_hash[&[9; 32]], vec![0, 1]);
        assert!(plan_is_empty(&[]));
    }

    #[test]
    fn a_suitcase_holds_at_most_so_many_blobs() {
        let entries: Vec<Entry> = (0..SUITCASE_FILES * 2 + 10)
            .map(|index| Entry {
                path: format!("{index:05}"),
                size: 10,
                mtime: 0,
                hash: blake3::hash(&index.to_le_bytes()).into(),
            })
            .collect();
        let plan = plan(&entries, &NoCipher, SUITCASE_BYTES);
        let runs: Vec<_> = plan.suitcases.iter().map(|s| s.entries.clone()).collect();
        let files = SUITCASE_FILES;
        assert_eq!(
            runs,
            vec![0..files, files..files * 2, files * 2..files * 2 + 10]
        );
    }

    fn plan_is_empty(entries: &[Entry]) -> bool {
        plan(entries, &NoCipher, 10).suitcases.is_empty()
    }
}
