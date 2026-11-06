//! "Move to another folder…" (gui-spec, Something you received): a
//! received stick's folder moves, with what came, into another place; what
//! came stays known as such. On the same disk it is renamed; onto another,
//! it is copied whole first (each file's time kept), and only then is the
//! old one deleted: a copy that fails deletes nothing of it. Blocking.

use std::path::{Path, PathBuf};

use crate::locks::Lock;
use crate::places::{check_received_folder, comparable, PlaceError};
use crate::stick_id::StickId;
use crate::store::{JoinedChange, Store, StoreError};

/// Why the folder did not move. Messages are for users.
#[derive(Debug, thiserror::Error)]
pub enum MoveError {
    #[error("it is not received here any more")]
    NotReceived,
    #[error("a folder called \u{201c}{name}\u{201d} is there already; choose another place")]
    AlreadyThere { name: String },
    #[error(transparent)]
    Place(#[from] PlaceError),
    #[error("it could not be moved: {source} ({})", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Moves the folder of the received stick `stick_id` into `into` (under its
/// own name) and records it there. `_held` is its download lock: the caller
/// has stopped receiving it. Returns where it is now.
pub fn move_received_folder(
    store: &dyn Store,
    data_dir: &Path,
    stick_id: &StickId,
    into: &Path,
    _held: &Lock,
) -> Result<PathBuf, MoveError> {
    let joined = store
        .joined_stick(stick_id)?
        .ok_or(MoveError::NotReceived)?;
    let from = joined.output_dir;
    let name = from.file_name().ok_or(MoveError::NotReceived)?.to_owned();
    let to = into.join(&name);
    if comparable(&to) == comparable(&from) {
        return Ok(from);
    }
    if std::fs::symlink_metadata(&to).is_ok() {
        return Err(MoveError::AlreadyThere {
            name: name.to_string_lossy().into_owned(),
        });
    }
    check_received_folder(store, data_dir, &to, stick_id)?;
    if comparable(&to).starts_with(comparable(&from)) {
        return Err(MoveError::Io {
            path: to,
            source: std::io::Error::other("a folder cannot go inside itself"),
        });
    }
    let moved =
        move_folder(&from, &to, &|from, to| std::fs::rename(from, to)).map_err(|source| {
            MoveError::Io {
                path: from.clone(),
                source,
            }
        })?;
    if let Err(error) = store.change_joined_stick(stick_id, &JoinedChange::Folder(to.clone())) {
        // Back to where the records say it is: renamed back, or the copy
        // removed (the old folder was left whole until now).
        match moved {
            Moved::Renamed => {
                if let Err(back) = std::fs::rename(&to, &from) {
                    tracing::warn!(%back, "a moved folder could not be put back");
                }
            }
            Moved::Copied(_) => {
                if let Err(back) = std::fs::remove_dir_all(&to) {
                    tracing::warn!(%back, "a copied folder could not be removed");
                }
            }
        }
        return Err(error.into());
    }
    if let Moved::Copied(copied) = moved {
        remove_copied(&from, &copied);
    }
    Ok(to)
}

/// How a folder went to its new place.
#[derive(Debug)]
pub(super) enum Moved {
    Renamed,
    /// Copied onto another disk: what was copied, files then folders, each
    /// folder after what is inside it. The old folder is still there.
    Copied(Vec<PathBuf>),
}

/// Removes from the old folder only what was copied: files, then folders
/// left empty. Anything else of the user's there (a link, a file made since)
/// stays, with the folders that hold it.
pub(super) fn remove_copied(from: &Path, copied: &[PathBuf]) {
    let mut kept = 0;
    for path in copied {
        let removed = if path.as_path() == from || path.is_dir() {
            std::fs::remove_dir(path)
        } else {
            std::fs::remove_file(path)
        };
        if removed.is_err() {
            kept += 1;
        }
    }
    if kept > 0 {
        tracing::warn!(kept, "the old folder keeps what was not moved");
    }
}

/// Moves `from` to `to` with `rename`; onto another disk, copies it whole
/// and leaves `from` for [`remove_copied`], once the move is recorded. A
/// copy that fails is removed and leaves `from` as it was.
pub(super) fn move_folder(
    from: &Path,
    to: &Path,
    rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<Moved> {
    match rename(from, to) {
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {}
        done => return done.map(|()| Moved::Renamed),
    }
    let mut copied = Vec::new();
    if let Err(error) = copy_tree(from, to, &mut copied) {
        // Only what this copy made: `to` did not exist before it.
        if std::fs::symlink_metadata(to).is_ok_and(|metadata| metadata.is_dir()) {
            let _ = std::fs::remove_dir_all(to);
        }
        return Err(error);
    }
    Ok(Moved::Copied(copied))
}

/// Copies the folder `from` to the new folder `to`: folders and files, each
/// file's time kept, and adds each to `copied` (a folder after what is
/// inside it). Links and other kinds are not received, so not copied. A
/// file keeps its mark from the internet (`std::fs::copy` copies it), and a
/// folder is given its own (ADR 0032, amendment 1).
fn copy_tree(from: &Path, to: &Path, copied: &mut Vec<PathBuf>) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    crate::download::files::copy_folder_mark(from, to);
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        if kind.is_dir() {
            copy_tree(&source, &target, copied)?;
        } else if kind.is_file() {
            std::fs::copy(&source, &target)?;
            let modified = entry.metadata()?.modified()?;
            std::fs::File::options()
                .write(true)
                .open(&target)?
                .set_modified(modified)?;
            copied.push(source);
        }
    }
    copied.push(from.to_path_buf());
    Ok(())
}

#[cfg(test)]
#[path = "relocate_tests.rs"]
mod tests;
