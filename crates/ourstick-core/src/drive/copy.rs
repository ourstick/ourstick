//! Copying onto a stick, as onto a real USB stick: all or nothing about
//! space, a name already there asked about first (ADR 0008; the drop's
//! words in `docs/gui-spec.md`), and the bytes told as they go.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::{free_name_where, io, remove_from, DriveError};
use crate::manifest::ignore::is_ignored;
use crate::share::folder_size;

/// Bytes are told after each piece this long.
const PIECE: usize = 1 << 20;

/// What to do with a dropped name already on the stick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnConflict {
    /// The new one gets " (2)", " (3)"… as file managers do.
    KeepBoth,
    /// The stick's copy goes and the new one takes its name. A linked
    /// entry is never replaced (its files are not the stick's): the new one
    /// is kept beside it.
    Replace,
}

/// How far a copy is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Copied {
    pub done: u64,
    pub total: u64,
}

/// The names of `sources` already on the stick as copies at its top level:
/// asked about before copying (Replace or Keep both). A linked entry's name
/// is not asked about: it is never replaced, the new one goes beside it.
pub fn names_taken(root: &Path, sources: &[PathBuf]) -> Vec<String> {
    sources
        .iter()
        .filter_map(|source| source.file_name()?.to_str())
        .filter(|name| std::fs::symlink_metadata(root.join(name)).is_ok())
        .map(str::to_owned)
        .collect()
}

/// Refuses a source that is on the stick already, or holds it, by where it
/// really is: through a link, or a drive letter that stands for the stick's
/// folder (Windows' `subst`), a path can look elsewhere. Replacing such a
/// source would delete what is being copied. Blocking.
pub fn check_sources(root: &Path, sources: &[PathBuf]) -> Result<(), DriveError> {
    let real_root = std::fs::canonicalize(root).map_err(io(root))?;
    for source in sources {
        let real = std::fs::canonicalize(source).map_err(io(source))?;
        if real.starts_with(&real_root) || real_root.starts_with(&real) {
            return Err(DriveError::NotCopyable {
                path: source.clone(),
                reason: "it is already on the stick",
            });
        }
    }
    Ok(())
}

/// A copy is made under this hidden name on the stick first, then takes its
/// own (sharing never sees it): a failed copy leaves nothing half done, and
/// what it replaces goes only once it is whole.
const COPYING: &str = ".ourstick-copying-";

/// Removes what a copy cut short left behind (only a copy's own names):
/// before the room left is measured, so they never take it. Blocking.
pub fn remove_leftovers(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(COPYING))
        {
            remove_any(&entry.path());
        }
    }
}

/// Removes a file or folder this module made, never following a link.
fn remove_any(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            let _ = std::fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(path);
        }
        Err(_) => {}
    }
}

/// The bytes of a file or folder to copy (system files left out).
fn source_size(path: &Path) -> Result<u64, DriveError> {
    let metadata = std::fs::symlink_metadata(path).map_err(io(path))?;
    if metadata.is_dir() {
        Ok(folder_size(path)?)
    } else if metadata.is_file() {
        Ok(metadata.len())
    } else {
        Err(DriveError::NotCopyable {
            path: path.to_path_buf(),
            reason: "only files and folders can go on a stick",
        })
    }
}

/// Copies one file in pieces, telling the bytes as they go.
fn copy_file(
    from: &Path,
    to: &Path,
    copied: &mut Copied,
    progress: &mut dyn FnMut(Copied),
) -> Result<(), DriveError> {
    let mut source = std::fs::File::open(from).map_err(io(from))?;
    let mut target = std::fs::File::create_new(to).map_err(io(to))?;
    let mut piece = vec![0; PIECE];
    loop {
        let read = source.read(&mut piece).map_err(io(from))?;
        if read == 0 {
            break;
        }
        target.write_all(&piece[..read]).map_err(io(to))?;
        copied.done += read as u64;
        progress(*copied);
    }
    // As `std::fs::copy` kept it: its permissions, and its time as it was.
    let metadata = source.metadata().map_err(io(from))?;
    if let Ok(modified) = metadata.modified() {
        target.set_modified(modified).map_err(io(to))?;
    }
    std::fs::set_permissions(to, metadata.permissions()).map_err(io(to))?;
    Ok(())
}

fn copy_tree(
    from: &Path,
    to: &Path,
    copied: &mut Copied,
    progress: &mut dyn FnMut(Copied),
) -> Result<(), DriveError> {
    let metadata = std::fs::symlink_metadata(from).map_err(io(from))?;
    if metadata.is_file() {
        return copy_file(from, to, copied, progress);
    }
    if !metadata.is_dir() {
        // Links inside a folder are skipped, as sharing skips them.
        return Ok(());
    }
    std::fs::create_dir(to).map_err(io(to))?;
    for entry in std::fs::read_dir(from).map_err(io(from))? {
        let entry = entry.map_err(io(from))?;
        let name = entry.file_name();
        if name.to_str().is_some_and(is_ignored) {
            continue;
        }
        copy_tree(&entry.path(), &to.join(&name), copied, progress)?;
    }
    Ok(())
}

/// Copies files and folders onto the stick, like onto a real USB stick; the
/// originals are not touched. All or nothing about space: if they need more
/// than the `left` bytes (what its disk has room for, from the space ledger,
/// ADR 0008), nothing is copied, as a real stick refuses; what a copy
/// replaces is not counted as room, since both are there until the new one
/// is whole. A name already on the stick is replaced or kept beside (" (2)")
/// as `on_conflict` says; a linked entry's name (`linked_names`, ignoring
/// case) is never replaced. Each copy is made under a hidden name and takes
/// its own only once whole. `progress` hears the bytes as they are copied.
/// Returns the names they got on the stick.
pub fn copy_onto(
    root: &Path,
    left: u64,
    linked_names: &[String],
    sources: &[PathBuf],
    on_conflict: OnConflict,
    progress: &mut dyn FnMut(Copied),
) -> Result<Vec<String>, DriveError> {
    check_sources(root, sources)?;
    let mut needed = 0;
    for source in sources {
        needed += source_size(source)?;
    }
    if needed > left {
        return Err(DriveError::DoesNotFit { needed, left });
    }
    remove_leftovers(root);
    let linked: Vec<String> = linked_names
        .iter()
        .map(|name| name.to_lowercase())
        .collect();
    let mut copied = Copied {
        done: 0,
        total: needed,
    };
    let mut names = Vec::new();
    for (number, source) in sources.iter().enumerate() {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| DriveError::NotCopyable {
                path: source.clone(),
                reason: "its name is not valid text",
            })?;
        let copying = root.join(format!("{COPYING}{number}"));
        if let Err(error) = copy_tree(source, &copying, &mut copied, progress) {
            remove_any(&copying);
            return Err(error);
        }
        let is_linked = linked.contains(&name.to_lowercase());
        if on_conflict == OnConflict::Replace
            && !is_linked
            && std::fs::symlink_metadata(root.join(name)).is_ok()
        {
            if let Err(error) = remove_from(root, name) {
                remove_any(&copying);
                return Err(error);
            }
        }
        let name = free_name_where(name, |candidate| {
            std::fs::symlink_metadata(root.join(candidate)).is_ok()
                || linked.contains(&candidate.to_lowercase())
        });
        let target = root.join(&name);
        if let Err(error) = std::fs::rename(&copying, &target) {
            remove_any(&copying);
            return Err(io(&target)(error));
        }
        names.push(name);
    }
    Ok(names)
}

#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
