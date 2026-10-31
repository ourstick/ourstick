//! Where a stick's files are on this device (ADR 0008): its own folder, and
//! the folders and files linked to it, each a top-level entry under its name.
//! A manifest path's first component picks its root. Paths never leave the
//! device; only names do.

use std::path::{Path, PathBuf};

use crate::sender_files::SenderFiles;
use crate::stick_id::StickId;
use crate::store::{LinkedEntry, Store, StoreError};

/// One root of a stick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    /// The name of a linked entry; `None` for the stick's own folder.
    pub name: Option<String>,
    pub path: PathBuf,
    pub is_dir: bool,
    /// The disk it must be on, if known.
    pub disk: Option<u64>,
    /// A linked folder whose contents are the stick's top level, without
    /// its name in front (ADR 0028 §2, amendment of 2026-10-20).
    pub at_top: bool,
}

impl From<&LinkedEntry> for Root {
    fn from(entry: &LinkedEntry) -> Self {
        Self {
            name: Some(entry.name.clone()),
            path: entry.path.clone(),
            is_dir: entry.is_dir,
            disk: entry.disk,
            at_top: entry.at_top,
        }
    }
}

impl Root {
    /// Gone: missing, a symbolic link now, no longer the same kind (folder or
    /// file), or on another disk than when linked (its disk is unplugged and
    /// an empty mount point is in its place). Blocking.
    pub fn is_gone(&self, files: &dyn SenderFiles) -> bool {
        match files.facts(&self.path) {
            Ok(facts) => {
                facts.is_folder() != self.is_dir
                    || (!self.is_dir && !facts.is_file())
                    || self
                        .disk
                        .is_some_and(|disk| facts.disk.is_some_and(|now| now != disk))
            }
            Err(_) => true,
        }
    }
}

/// A stick's roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickLayout {
    own: Root,
    linked: Vec<Root>,
}

impl StickLayout {
    /// The stick whose own folder is `own`, with `linked` entries.
    pub fn new(own: &Path, linked: &[LinkedEntry]) -> Self {
        Self {
            own: Root {
                name: None,
                path: own.to_path_buf(),
                is_dir: true,
                disk: None,
                at_top: false,
            },
            linked: linked.iter().map(Root::from).collect(),
        }
    }

    /// The layout of an owned stick, as the store has it now. Blocking.
    pub fn load(store: &dyn Store, stick_id: &StickId) -> Result<Self, StoreError> {
        let stick = store
            .owned_stick(stick_id)?
            .ok_or(StoreError::StickNotFound)?;
        Ok(Self::new(
            &stick.root_path,
            &store.linked_entries(stick_id)?,
        ))
    }

    pub fn own(&self) -> &Root {
        &self.own
    }

    pub fn linked(&self) -> &[Root] {
        &self.linked
    }

    /// The linked root named exactly `name` in the manifest (not the one at
    /// the top, whose name is not there).
    fn linked_named(&self, name: &str) -> Option<&Root> {
        self.linked
            .iter()
            .find(|root| !root.at_top && root.name.as_deref() == Some(name))
    }

    /// The linked folder at the top, if the stick has one.
    pub fn top(&self) -> Option<&Root> {
        self.linked.iter().find(|root| root.at_top)
    }

    /// Whether the manifest's top-level name `name` is a linked entry's own.
    pub fn is_named(&self, name: &str) -> bool {
        self.linked_named(name).is_some()
    }

    /// The root a manifest path is under: a named linked entry by its first
    /// component, else the folder at the top, else the own folder.
    pub fn root_of(&self, relative: &str) -> &Root {
        let first = relative.split('/').next().unwrap_or_default();
        self.linked_named(first)
            .or_else(|| self.top())
            .unwrap_or(&self.own)
    }

    /// Where a manifest path is on disk.
    pub fn on_disk(&self, relative: &str) -> PathBuf {
        let mut parts = relative.split('/');
        let first = parts.next().unwrap_or_default();
        let (root, parts) = match self.linked_named(first) {
            Some(root) => (root, parts),
            None => (self.top().unwrap_or(&self.own), relative.split('/')),
        };
        // One path, grown in place (a join per part would make a new one).
        let mut path = PathBuf::with_capacity(root.path.as_os_str().len() + relative.len() + 1);
        path.push(&root.path);
        for part in parts {
            path.push(part);
        }
        path
    }

    /// Whether a top-level entry of the own folder, or of the folder at the
    /// top, is hidden by a named linked entry of the same name, ignoring
    /// case: the linked one keeps the name (it was there first, and readers
    /// may have it half-downloaded).
    pub fn is_shadowed(&self, top_name: &str) -> bool {
        let lower = top_name.to_lowercase();
        self.linked.iter().any(|root| {
            !root.at_top
                && root
                    .name
                    .as_deref()
                    .is_some_and(|name| name.to_lowercase() == lower)
        })
    }
}

#[cfg(test)]
mod tests;
