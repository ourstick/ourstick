//! What a plan writes, in bytes: for the space ledger and a preview.

use std::collections::BTreeMap;

use super::{FilePlan, FolderPlan};
use crate::manifest::Manifest;

impl FolderPlan {
    /// The bytes this plan writes, by each top-level entry of the stick
    /// and each entry inside a top-level folder they are under (a preview's
    /// per-entry "needs up to", ADR 0012 and its Amendment 1): keys of one
    /// part and of two.
    pub(in crate::download) fn bytes_by_top(&self, manifest: &Manifest) -> BTreeMap<String, u64> {
        let mut by_path: BTreeMap<String, u64> = BTreeMap::new();
        for (plan, entry) in self.files.iter().zip(&manifest.entries) {
            if matches!(plan, FilePlan::Download { .. }) {
                let mut parts = entry.path.splitn(3, '/');
                let first = parts.next().unwrap_or_default();
                let mut keys = vec![first.to_owned()];
                if let Some(second) = parts.next() {
                    keys.push(format!("{first}/{second}"));
                }
                for key in keys {
                    let bytes = by_path.entry(key).or_default();
                    *bytes = bytes.saturating_add(entry.size);
                }
            }
        }
        by_path
    }

    /// The sizes of the files this plan downloads.
    fn sizes_to_write<'m>(&'m self, manifest: &'m Manifest) -> impl Iterator<Item = u64> + 'm {
        self.files
            .iter()
            .zip(&manifest.entries)
            .filter(|(plan, _)| matches!(plan, FilePlan::Download { .. }))
            .map(|(_, entry)| entry.size)
    }

    /// The largest file this plan writes into the folder.
    pub(in crate::download) fn largest_to_write(&self, manifest: &Manifest) -> u64 {
        self.sizes_to_write(manifest).max().unwrap_or(0)
    }

    /// The bytes this plan writes into the folder: the files to download.
    pub(in crate::download) fn bytes_to_write(&self, manifest: &Manifest) -> u64 {
        self.sizes_to_write(manifest).sum()
    }
}
