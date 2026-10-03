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

/// The names of `sources` already on the stick: a copy at its top level,
/// or a linked entry's name (ignoring case). Asked about before copying.
pub fn names_taken(root: &Path, linked_names: &[String], sources: &[PathBuf]) -> Vec<String> {
    sources
        .iter()
        .filter_map(|source| source.file_name()?.to_str())
        .filter(|name| {
            std::fs::symlink_metadata(root.join(name)).is_ok()
                || linked_names
                    .iter()
                    .any(|linked| linked.to_lowercase() == name.to_lowercase())
        })
        .map(str::to_owned)
        .collect()
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
    let permissions = source.metadata().map_err(io(from))?.permissions();
    std::fs::set_permissions(to, permissions).map_err(io(to))?;
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
/// ADR 0008), nothing is copied, as a real stick refuses. A name already on
/// the stick, or a linked entry's (`linked_names`, ignoring case), is
/// replaced or kept beside as `on_conflict` says; `progress` hears the bytes
/// as they are copied. Returns the names they got on the stick.
pub fn copy_onto(
    root: &Path,
    left: u64,
    linked_names: &[String],
    sources: &[PathBuf],
    on_conflict: OnConflict,
    progress: &mut dyn FnMut(Copied),
) -> Result<Vec<String>, DriveError> {
    let mut needed = 0;
    for source in sources {
        if source.starts_with(root) {
            return Err(DriveError::NotCopyable {
                path: source.clone(),
                reason: "it is already on the stick",
            });
        }
        needed += source_size(source)?;
    }
    let linked: Vec<String> = linked_names
        .iter()
        .map(|name| name.to_lowercase())
        .collect();
    // What is replaced makes room for what replaces it.
    let mut freed = 0;
    if on_conflict == OnConflict::Replace {
        for name in names_taken(root, &[], sources) {
            if !linked.contains(&name.to_lowercase()) {
                freed += source_size(&root.join(&name)).unwrap_or(0);
            }
        }
    }
    if needed > left.saturating_add(freed) {
        return Err(DriveError::DoesNotFit { needed, left });
    }
    let mut copied = Copied {
        done: 0,
        total: needed,
    };
    let mut names = Vec::new();
    for source in sources {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| DriveError::NotCopyable {
                path: source.clone(),
                reason: "its name is not valid text",
            })?;
        let is_linked = linked.contains(&name.to_lowercase());
        if on_conflict == OnConflict::Replace
            && !is_linked
            && std::fs::symlink_metadata(root.join(name)).is_ok()
        {
            remove_from(root, name)?;
        }
        let name = free_name_where(name, |candidate| {
            root.join(candidate).exists() || linked.contains(&candidate.to_lowercase())
        });
        copy_tree(source, &root.join(&name), &mut copied, progress)?;
        names.push(name);
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![7u8; bytes]).unwrap();
    }

    #[test]
    fn names_already_on_the_stick_are_told_before_copying() {
        let stick = tempfile::tempdir().unwrap();
        let from = tempfile::tempdir().unwrap();
        write(&stick.path().join("new in/a.jpg"), 10);
        for name in ["new in/b.jpg", "Films/c.mkv", "notes.txt"] {
            write(&from.path().join(name), 10);
        }
        let sources: Vec<PathBuf> = ["new in", "Films", "notes.txt"]
            .iter()
            .map(|name| from.path().join(name))
            .collect();
        // A copy there, and a linked entry's name in another case.
        let taken = names_taken(stick.path(), &["films".to_owned()], &sources);
        assert_eq!(taken, ["new in", "Films"]);
    }

    #[test]
    fn replace_takes_the_copys_place_and_never_a_linked_entrys() {
        let stick = tempfile::tempdir().unwrap();
        let from = tempfile::tempdir().unwrap();
        write(&stick.path().join("new in/old.jpg"), 50);
        write(&from.path().join("new in/new.jpg"), 60);
        write(&from.path().join("Films/c.mkv"), 5);
        let sources = [from.path().join("new in"), from.path().join("Films")];
        // 65 bytes needed, 20 left: the 50 replaced make room.
        let names = copy_onto(
            stick.path(),
            20,
            &["Films".to_owned()],
            &sources,
            OnConflict::Replace,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(names, ["new in", "Films (2)"]);
        assert!(!stick.path().join("new in/old.jpg").exists());
        assert!(stick.path().join("new in/new.jpg").exists());
        // The originals are where they were.
        assert!(from.path().join("new in/new.jpg").exists());
    }

    #[test]
    fn the_bytes_are_told_as_they_are_copied() {
        let stick = tempfile::tempdir().unwrap();
        let from = tempfile::tempdir().unwrap();
        write(&from.path().join("big.bin"), 3 * PIECE + 5);
        write(&from.path().join("folder/small.txt"), 10);
        let mut told = Vec::new();
        copy_onto(
            stick.path(),
            u64::MAX,
            &[],
            &[from.path().join("big.bin"), from.path().join("folder")],
            OnConflict::KeepBoth,
            &mut |copied| told.push(copied),
        )
        .unwrap();
        let total = (3 * PIECE + 15) as u64;
        assert!(told.iter().all(|copied| copied.total == total));
        assert!(told.windows(2).all(|pair| pair[0].done < pair[1].done));
        assert_eq!(told.last().unwrap().done, total);
        assert!(told.len() >= 5);
    }
}
