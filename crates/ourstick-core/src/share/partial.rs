//! What a partial version lists (ADR 0007). While a pass reads the folder, a
//! partial version holds the files read so far, and, after them in manifest
//! order, the files of the last pass that are still in the folder and not
//! read yet, as they were then. A reader of an earlier version then sees no
//! file leave the stick only because this pass has not reached it; a file
//! that did leave is gone at once.

use std::sync::Arc;

use crate::manifest::{Entry, Manifest};

/// The files of the last pass, and what was last published.
#[derive(Debug)]
pub(super) struct Previous {
    /// In manifest order.
    entries: Vec<Entry>,
    /// What was last published, the manifest the publisher keeps (ADR
    /// 0019); `None`: `entries`, not yet another (one copy, not two, of a
    /// list that may be long).
    published: Option<Arc<Manifest>>,
}

impl Previous {
    /// `recorded` is what the last pass recorded of the files this pass
    /// found, in manifest order ([`crate::manifest::Catalogue::recorded_entries`]):
    /// a recorded file no longer found is not in it.
    pub(super) fn new(recorded: Vec<Entry>) -> Self {
        Self {
            entries: recorded,
            published: None,
        }
    }

    /// The entries of a partial version: `read` (this pass so far, through
    /// the path `last_path`), then the earlier files after `last_path`.
    pub(super) fn merged(&self, read: &[Entry], last_path: Option<&str>) -> Vec<Entry> {
        let after = last_path.map_or(0, |last| {
            self.entries
                .partition_point(|entry| entry.path.as_str() <= last)
        });
        let mut merged = Vec::with_capacity(read.len() + self.entries.len() - after);
        merged.extend_from_slice(read);
        merged.extend_from_slice(&self.entries[after..]);
        merged
    }

    /// Whether `merged` differs from what was last published (at first, the
    /// last pass's files): a partial version that changes nothing is not
    /// published.
    pub(super) fn changes(&self, merged: &[Entry]) -> bool {
        self.published
            .as_ref()
            .map_or(&self.entries, |manifest| &manifest.entries)
            != merged
    }

    pub(super) fn published(&mut self, manifest: Arc<Manifest>) {
        self.published = Some(manifest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, fill: u8) -> Entry {
        Entry {
            path: path.to_owned(),
            size: 10,
            mtime: 1_700_000_000,
            hash: [fill; 32],
        }
    }

    fn paths(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.path.as_str()).collect()
    }

    #[test]
    fn files_not_read_yet_stay_as_they_were_and_gone_ones_leave() {
        // "gone" is not in what the catalogue gives (not found this pass).
        let previous = Previous::new(vec![
            entry("a", 1),
            entry("b", 2),
            entry("c", 3),
            entry("d", 4),
        ]);
        // Nothing read yet: the last pass's files that are still there.
        let at_start = previous.merged(&[], None);
        assert_eq!(paths(&at_start), vec!["a", "b", "c", "d"]);
        assert_eq!(at_start[0], entry("a", 1));
        assert!(!previous.changes(&at_start));

        // "a" and "b" read, "b" changed: "c" and "d" as they were.
        let merged = previous.merged(&[entry("a", 1), entry("b", 7)], Some("b"));
        assert_eq!(paths(&merged), vec!["a", "b", "c", "d"]);
        assert_eq!(merged[1].hash, [7; 32]);
        assert!(previous.changes(&merged));
    }

    #[test]
    fn a_file_skipped_by_this_pass_leaves_the_partial_version() {
        let mut previous = Previous::new(vec![entry("a", 1), entry("b", 2), entry("c", 3)]);
        // "b" was reached but left out (changing while read).
        let merged = previous.merged(&[entry("a", 1)], Some("b"));
        assert_eq!(paths(&merged), vec!["a", "c"]);
        assert!(previous.changes(&merged));
        previous.published(Arc::new(Manifest {
            partial: true,
            name: String::new(),
            entries: merged.clone(),
        }));
        assert!(!previous.changes(&merged));
    }
}
