//! What a partial version lists (ADR 0007). While a pass reads the folder, a
//! partial version holds the files read so far, and, after them in manifest
//! order, the files of the last pass that are still in the folder and not
//! read yet, as they were then. A reader of an earlier version then sees no
//! file leave the stick only because this pass has not reached it; a file
//! that did leave is gone at once.

use std::collections::{BTreeMap, HashSet};
use std::ops::Bound;

use crate::manifest::Entry;
use crate::store::OwnedFile;

/// The files of the last pass, and what was last published.
#[derive(Debug)]
pub(super) struct Previous {
    entries: BTreeMap<String, Entry>,
    published: Vec<Entry>,
}

impl Previous {
    /// `recorded` is what the last pass recorded; `walked` every path this
    /// pass found. A recorded file no longer found is left out.
    pub(super) fn new(recorded: Vec<OwnedFile>, walked: &[String]) -> Self {
        let walked: HashSet<&str> = walked.iter().map(String::as_str).collect();
        let entries: BTreeMap<String, Entry> = recorded
            .into_iter()
            .filter(|file| walked.contains(file.path.as_str()))
            .map(|file| {
                let entry = Entry {
                    path: file.path.clone(),
                    size: file.stamp.size,
                    // Whole seconds, as the catalogue records them.
                    mtime: file.stamp.mtime_ns / 1_000_000_000,
                    hash: file.blob_hash,
                };
                (file.path, entry)
            })
            .collect();
        let published = entries.values().cloned().collect();
        Self { entries, published }
    }

    /// The entries of a partial version: `read` (this pass so far, through
    /// the path `last_path`), then the earlier files after `last_path`.
    pub(super) fn merged(&self, read: &[Entry], last_path: Option<&str>) -> Vec<Entry> {
        let after = match last_path {
            Some(path) => Bound::Excluded(path),
            None => Bound::Unbounded,
        };
        let mut merged = read.to_vec();
        merged.extend(
            self.entries
                .range::<str, _>((after, Bound::Unbounded))
                .map(|(_, entry)| entry.clone()),
        );
        merged
    }

    /// Whether `merged` differs from what was last published (at first, the
    /// last pass's files): a partial version that changes nothing is not
    /// published.
    pub(super) fn changes(&self, merged: &[Entry]) -> bool {
        self.published != merged
    }

    pub(super) fn published(&mut self, merged: Vec<Entry>) {
        self.published = merged;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::FileStamp;

    fn recorded(path: &str, fill: u8) -> OwnedFile {
        OwnedFile {
            path: path.to_owned(),
            stamp: FileStamp {
                size: 10,
                mtime_ns: 1_700_000_000_900_000_000,
                ctime_ns: 0,
            },
            blob_hash: [fill; 32],
            plaintext_hash: None,
        }
    }

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
        let walked: Vec<String> = ["a", "b", "c", "d", "new"].map(String::from).to_vec();
        let previous = Previous::new(
            vec![
                recorded("a", 1),
                recorded("b", 2),
                recorded("c", 3),
                recorded("gone", 9),
                recorded("d", 4),
            ],
            &walked,
        );
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
        let walked: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let mut previous = Previous::new(
            vec![recorded("a", 1), recorded("b", 2), recorded("c", 3)],
            &walked,
        );
        // "b" was reached but left out (changing while read).
        let merged = previous.merged(&[entry("a", 1)], Some("b"));
        assert_eq!(paths(&merged), vec!["a", "c"]);
        assert!(previous.changes(&merged));
        previous.published(merged.clone());
        assert!(!previous.changes(&merged));
    }
}
