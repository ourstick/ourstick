//! Folders and files shared where they are, not copied onto the stick
//! (ADR 0008 §2): the one place that decides whether a path may be linked,
//! and under which name. Blocking I/O; call from `spawn_blocking`.

use std::path::{Path, PathBuf};

use crate::places::{app_places, clash, Taken};
use crate::sender_files::{FileKind, SenderFiles};
use crate::stick_id::StickId;
use crate::store::{LinkedEntry, Store, StoreError};

/// Why a path cannot be linked. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("{} is not a full path", .0.display())]
    NotAbsolute(PathBuf),
    #[error("{} is a shortcut (a symbolic link); share what it points to instead", .0.display())]
    SymbolicLink(PathBuf),
    #[error("{} is neither a file nor a folder", .0.display())]
    NotFileOrFolder(PathBuf),
    #[error("{}'s name cannot be shared (it is not valid Unicode, or not a name at all)", .0.display())]
    BadName(PathBuf),
    #[error("{}", name_taken(.name, .path, .other.as_deref()))]
    NameTaken {
        path: PathBuf,
        name: String,
        /// The linked entry that has the name, if one has (else the stick's
        /// own folder has it).
        other: Option<PathBuf>,
    },
    #[error("\u{201c}{name}\u{201d} is already in \u{201c}{top}\u{201d}. Rename one first, then add it again.")]
    InTop { name: String, top: String },
    #[error("\u{201c}{0}\u{201d} is not found, so OurStick cannot check the names in it.")]
    TopNotFound(String),
    #[error("{} {relation} {other}, which a link may not touch", .path.display())]
    Overlaps {
        path: PathBuf,
        relation: &'static str,
        other: String,
    },
    #[error("I/O error on {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// "Two things are called “Photos” (from “Pictures” and from “Expansion”).
/// Rename one first, then add it again."
fn name_taken(name: &str, path: &Path, other: Option<&Path>) -> String {
    let from = |path: &Path| {
        path.parent()
            .and_then(Path::file_name)
            .map(|parent| format!("\u{201c}{}\u{201d}", parent.to_string_lossy()))
    };
    let whence = match other.map(|other| (from(other), from(path))) {
        Some((Some(other), Some(this))) if other != this => {
            format!(" (from {other} and from {this})")
        }
        _ => String::new(),
    };
    format!(
        "Two things are called \u{201c}{name}\u{201d}{whence}. Rename one first, then add it again."
    )
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> LinkError + '_ {
    move |source| LinkError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The disk a file is on (`st_dev`), where the system tells.
pub(crate) fn disk_of(metadata: &std::fs::Metadata) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(metadata.dev())
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

/// Decides whether `path` may be linked to the stick `stick_id`, and its
/// name there. Refused: a symbolic link; anything that is, contains or lies
/// inside this stick's own folder or one of its links (each file once in a
/// stick), and the places only OurStick may use ([`crate::places`]: other
/// app sticks' folders, the data folder). `planned` are links decided but
/// not stored yet (several dropped at once). The name is the last component
/// of `path`; one that clashes, ignoring case, with an entry already on the
/// stick is refused, never numbered (ADR 0028 §2: the receiver would not
/// know which is which). `now_ms` is when it is added.
pub fn plan_link(
    files: &dyn SenderFiles,
    store: &dyn Store,
    stick_id: &StickId,
    data_dir: &Path,
    planned: &[LinkedEntry],
    path: &Path,
    now_ms: u64,
) -> Result<LinkedEntry, LinkError> {
    let stick = store
        .owned_stick(stick_id)?
        .ok_or(StoreError::StickNotFound)?;
    let own = stick.root_path.as_path();
    let mut existing = store.linked_entries(stick_id)?;
    existing.extend(planned.iter().cloned());
    if !path.is_absolute() {
        return Err(LinkError::NotAbsolute(path.to_path_buf()));
    }
    let facts = files.facts(path).map_err(io(path))?;
    match facts.kind {
        FileKind::Link => return Err(LinkError::SymbolicLink(path.to_path_buf())),
        FileKind::Other => return Err(LinkError::NotFileOrFolder(path.to_path_buf())),
        FileKind::File | FileKind::Folder => {}
    }
    let mut taken = vec![Taken {
        place: crate::places::comparable(own),
        what: "the stick's own folder".to_owned(),
    }];
    taken.extend(existing.iter().map(|entry| Taken {
        place: crate::places::comparable(&entry.path),
        what: format!("\u{201c}{}\u{201d}, already shared", entry.name),
    }));
    taken.extend(app_places(store, data_dir, Some(stick_id))?);
    if let Some((relation, taken)) = clash(path, &taken) {
        return Err(LinkError::Overlaps {
            path: path.to_path_buf(),
            relation: relation.words(),
            other: taken.what.clone(),
        });
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.contains('\\'))
        .ok_or_else(|| LinkError::BadName(path.to_path_buf()))?;
    let on_stick = top_names(files, own)?;
    // The entry at the top keeps its name too, though it is not in the
    // manifest: two entries of one stick never share a name.
    if let Some(refused) = name_clash(
        path,
        existing
            .iter()
            .map(|entry| (entry.name.as_str(), entry.path.as_path())),
    ) {
        return Err(refused);
    }
    if let Some(top) = existing.iter().find(|entry| entry.at_top) {
        check_next_to_top(files, top, name)?;
    }
    let lower = name.to_lowercase();
    if on_stick.iter().any(|name| name.to_lowercase() == lower)
        // What the file system itself calls the same (e.g. Unicode forms).
        || files.facts_followed(&own.join(name)).is_ok()
    {
        return Err(LinkError::NameTaken {
            path: path.to_path_buf(),
            name: name.to_owned(),
            other: None,
        });
    }
    Ok(LinkedEntry {
        name: name.to_owned(),
        path: path.to_path_buf(),
        is_dir: facts.is_folder(),
        disk: facts.disk,
        added_at_ms: now_ms,
        at_top: false,
    })
}

/// Refuses `name` next to `top`, the folder at a stick's top: a name in it
/// already, ignoring case, or any while it is not found (its names cannot
/// be checked). Blocking.
pub fn check_next_to_top(
    files: &dyn SenderFiles,
    top: &LinkedEntry,
    name: &str,
) -> Result<(), LinkError> {
    if crate::layout::Root::from(top).is_gone(files) {
        return Err(LinkError::TopNotFound(top.name.clone()));
    }
    let lower = name.to_lowercase();
    if top_names(files, &top.path)?
        .iter()
        .any(|inside| inside.to_lowercase() == lower)
        // What the file system itself calls the same (e.g. Unicode forms).
        || files.facts_followed(&top.path.join(name)).is_ok()
    {
        return Err(LinkError::InTop {
            name: name.to_owned(),
            top: top.name.clone(),
        });
    }
    Ok(())
}

/// A new stick's entries, as planned: if they are exactly one folder, it
/// goes at the top, its contents the stick's top level (ADR 0028 §2,
/// amendment of 2026-10-20), so a receiver does not get the folder inside a
/// folder of the same name. Decided once, when the stick is made.
pub fn one_folder_at_top(planned: &mut [LinkedEntry]) {
    if let [only] = planned {
        only.at_top = only.is_dir;
    }
}

/// The refusal of `path` if one of `others` (name, path) has its name,
/// ignoring case: the receiver would not know which is which (ADR 0028 §2).
pub fn name_clash<'a>(
    path: &Path,
    others: impl IntoIterator<Item = (&'a str, &'a Path)>,
) -> Option<LinkError> {
    let name = path.file_name()?.to_string_lossy();
    let lower = name.to_lowercase();
    others
        .into_iter()
        .find(|(other, _)| other.to_lowercase() == lower)
        .map(|(_, other)| LinkError::NameTaken {
            path: path.to_path_buf(),
            name: name.into_owned(),
            other: Some(other.to_path_buf()),
        })
}

/// The names in the stick's own folder (none if it is not there, e.g. an
/// ejected disk image).
fn top_names(files: &dyn SenderFiles, own: &Path) -> Result<Vec<String>, LinkError> {
    match files.entries(own) {
        Ok(entries) => Ok(entries
            .into_iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(io(own)(source)),
    }
}

#[cfg(test)]
mod tests;
