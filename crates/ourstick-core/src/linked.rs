//! Folders and files shared where they are, not copied onto the stick
//! (ADR 0008 §2): the one place that decides whether a path may be linked,
//! and under which name. Blocking I/O; call from `spawn_blocking`.

use std::path::{Path, PathBuf};

use crate::drive::free_name_where;
use crate::places::{app_places, clash, Taken};
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
/// of `path`, with " (2)", " (3)"… if it clashes, ignoring case, with an
/// entry already on the stick. `now_ms` is when it is added.
pub fn plan_link(
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
    let metadata = std::fs::symlink_metadata(path).map_err(io(path))?;
    if metadata.file_type().is_symlink() {
        return Err(LinkError::SymbolicLink(path.to_path_buf()));
    }
    if !metadata.is_dir() && !metadata.is_file() {
        return Err(LinkError::NotFileOrFolder(path.to_path_buf()));
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
    let on_stick = top_names(own)?;
    let taken = |candidate: &str| {
        let lower = candidate.to_lowercase();
        on_stick.iter().any(|name| name.to_lowercase() == lower)
            || existing.iter().any(|entry| entry.name.to_lowercase() == lower)
            // What the file system itself calls the same (e.g. Unicode forms).
            || own.join(candidate).exists()
    };
    Ok(LinkedEntry {
        name: free_name_where(name, taken),
        path: path.to_path_buf(),
        is_dir: metadata.is_dir(),
        disk: disk_of(&metadata),
        added_at_ms: now_ms,
    })
}

/// The names in the stick's own folder (none if it is not there, e.g. an
/// ejected disk image).
fn top_names(own: &Path) -> Result<Vec<String>, LinkError> {
    let entries = match std::fs::read_dir(own) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io(own)(source)),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io(own))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    Ok(names)
}

#[cfg(test)]
mod tests;
