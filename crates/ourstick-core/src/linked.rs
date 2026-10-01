//! Folders and files shared where they are, not copied onto the stick
//! (ADR 0008 §2): the one place that decides whether a path may be linked,
//! and under which name. Blocking I/O; call from `spawn_blocking`.

use std::path::{Path, PathBuf};

use crate::drive::free_name_where;
use crate::store::LinkedEntry;

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
    #[error("{} {relation} {other}; a stick shares each file once", .path.display())]
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

/// `path` with links resolved, or as it is if it cannot be (gone).
fn real(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `a` contains `b`, `b` contains `a`, or they are one.
fn relation(a: &Path, b: &Path) -> Option<&'static str> {
    if a == b {
        Some("is")
    } else if b.starts_with(a) {
        Some("contains")
    } else if a.starts_with(b) {
        Some("is inside")
    } else {
        None
    }
}

/// Decides whether `path` may be linked to the stick whose own folder is
/// `own`, and its name there. Refused: a symbolic link; anything that is,
/// contains or lies inside the stick's folder, the data folder (`data_dir`,
/// which holds the cache) or another linked entry. The name is the last
/// component of `path`, with " (2)", " (3)"… if it clashes, ignoring case,
/// with an entry already on the stick. `now_ms` is when it is added.
pub fn plan_link(
    own: &Path,
    data_dir: &Path,
    existing: &[LinkedEntry],
    path: &Path,
    now_ms: u64,
) -> Result<LinkedEntry, LinkError> {
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
    let resolved = std::fs::canonicalize(path).map_err(io(path))?;
    let mut others = vec![
        (real(own), "the stick's own folder".to_owned()),
        (real(data_dir), "OurStick's data folder".to_owned()),
    ];
    others.extend(existing.iter().map(|entry| {
        (
            real(&entry.path),
            format!("\u{201c}{}\u{201d}, already shared", entry.name),
        )
    }));
    for (other_path, other) in others {
        if let Some(relation) = relation(&resolved, &other_path) {
            return Err(LinkError::Overlaps {
                path: path.to_path_buf(),
                relation,
                other,
            });
        }
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
