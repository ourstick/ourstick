//! Where a stick's files are on this device (ADR 0008): its own folder, and
//! the folders and files linked to it, each a top-level entry under its name.
//! A manifest path's first component picks its root. Paths never leave the
//! device; only names do.

use std::path::{Path, PathBuf};

use crate::linked::disk_of;
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
}

impl Root {
    /// Gone: missing, a symbolic link now, no longer the same kind (folder or
    /// file), or on another disk than when linked (its disk is unplugged and
    /// an empty mount point is in its place). Blocking.
    pub fn is_gone(&self) -> bool {
        match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) => {
                metadata.file_type().is_symlink()
                    || metadata.is_dir() != self.is_dir
                    || (!self.is_dir && !metadata.is_file())
                    || self
                        .disk
                        .is_some_and(|disk| disk_of(&metadata).is_some_and(|now| now != disk))
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
            },
            linked: linked
                .iter()
                .map(|entry| Root {
                    name: Some(entry.name.clone()),
                    path: entry.path.clone(),
                    is_dir: entry.is_dir,
                    disk: entry.disk,
                })
                .collect(),
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

    /// The linked root named exactly `name`.
    fn linked_named(&self, name: &str) -> Option<&Root> {
        self.linked
            .iter()
            .find(|root| root.name.as_deref() == Some(name))
    }

    /// The root a manifest path is under.
    pub fn root_of(&self, relative: &str) -> &Root {
        let first = relative.split('/').next().unwrap_or_default();
        self.linked_named(first).unwrap_or(&self.own)
    }

    /// Where a manifest path is on disk.
    pub fn on_disk(&self, relative: &str) -> PathBuf {
        let mut parts = relative.split('/');
        let first = parts.next().unwrap_or_default();
        match self.linked_named(first) {
            Some(root) => parts.fold(root.path.clone(), |path, part| path.join(part)),
            None => relative
                .split('/')
                .fold(self.own.path.clone(), |path, part| path.join(part)),
        }
    }

    /// Whether a top-level entry of the own folder is hidden by a linked
    /// entry of the same name, ignoring case: the linked one keeps the name
    /// (it was there first, and readers may have it half-downloaded).
    pub fn is_shadowed(&self, own_top_name: &str) -> bool {
        let lower = own_top_name.to_lowercase();
        self.linked.iter().any(|root| {
            root.name
                .as_deref()
                .is_some_and(|name| name.to_lowercase() == lower)
        })
    }
}

#[cfg(test)]
mod tests;
