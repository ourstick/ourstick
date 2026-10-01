//! The app's stick as a folder (docs/gui-spec.md): what is on its top level,
//! copying files and folders onto it as onto a real USB stick, and removing
//! them again. Blocking I/O throughout; call from `spawn_blocking`.

use std::path::{Path, PathBuf};

use crate::manifest::ignore::is_ignored;
use crate::share::{folder_size, ShareError};

/// Why a copy onto the stick or a removal failed. Messages are for users.
#[derive(Debug, thiserror::Error)]
pub enum DriveError {
    #[error("the stick is full: {needed} bytes do not fit in the {free} left")]
    Full { needed: u64, free: u64 },
    #[error("{} cannot be copied: {reason}", .path.display())]
    NotCopyable { path: PathBuf, reason: &'static str },
    #[error("{name} is not on the stick")]
    NotOnStick { name: String },
    #[error("I/O error on {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Share(#[from] ShareError),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> DriveError + '_ {
    move |source| DriveError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// One entry on the stick's top level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopEntry {
    pub name: String,
    /// For a folder, the size of everything in it.
    pub size: u64,
    pub is_dir: bool,
}

/// The stick's top level, sorted by name, system files left out.
pub fn top_entries(root: &Path) -> Result<Vec<TopEntry>, DriveError> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(root).map_err(io(root))? {
        let entry = entry.map_err(io(root))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if is_ignored(&name) {
            continue;
        }
        let file_type = entry.file_type().map_err(io(&entry.path()))?;
        let (size, is_dir) = if file_type.is_dir() {
            (folder_size(&entry.path())?, true)
        } else if file_type.is_file() {
            (entry.metadata().map_err(io(&entry.path()))?.len(), false)
        } else {
            // Links and other kinds are never shared; do not list them.
            continue;
        };
        entries.push(TopEntry { name, size, is_dir });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// A name for `name` on the stick that is not taken: `name`, else
/// `stem (2).ext`, `stem (3).ext`, … as file managers do.
fn free_name(root: &Path, name: &str) -> String {
    free_name_where(name, |candidate| root.join(candidate).exists())
}

/// `name`, else `stem (2).ext`, `stem (3).ext`, …: the first that `taken`
/// does not refuse.
pub(crate) fn free_name_where(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_owned();
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    (2..)
        .map(|number| format!("{stem} ({number}){extension}"))
        .find(|candidate| !taken(candidate))
        .expect("some number is free")
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

fn copy_tree(from: &Path, to: &Path) -> Result<(), DriveError> {
    let metadata = std::fs::symlink_metadata(from).map_err(io(from))?;
    if metadata.is_file() {
        std::fs::copy(from, to).map_err(io(to))?;
        return Ok(());
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
        copy_tree(&entry.path(), &to.join(&name))?;
    }
    Ok(())
}

/// Copies files and folders onto the stick, like onto a real USB stick; the
/// originals are not touched. All or nothing about space: if they do not fit
/// in `size` together with what is there, nothing is copied. Returns the
/// names they got on the stick.
pub fn copy_onto(root: &Path, size: u64, sources: &[PathBuf]) -> Result<Vec<String>, DriveError> {
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
    let used = folder_size(root)?;
    let free = size.saturating_sub(used);
    if needed > free {
        return Err(DriveError::Full { needed, free });
    }
    let mut names = Vec::new();
    for source in sources {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| DriveError::NotCopyable {
                path: source.clone(),
                reason: "its name is not valid text",
            })?;
        let name = free_name(root, name);
        copy_tree(source, &root.join(&name))?;
        names.push(name);
    }
    Ok(names)
}

/// Removes a file or folder from the stick's top level (the stick's copy).
pub fn remove_from(root: &Path, name: &str) -> Result<(), DriveError> {
    let not_on_stick = || DriveError::NotOnStick {
        name: name.to_owned(),
    };
    // Only a plain name on the top level: never a path that leads elsewhere.
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(not_on_stick());
    }
    let path = root.join(name);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| not_on_stick())?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(&path).map_err(io(&path))
    } else {
        std::fs::remove_file(&path).map_err(io(&path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![1u8; bytes]).unwrap();
    }

    #[test]
    fn the_top_level_lists_files_and_folders_with_sizes() {
        let stick = tempfile::tempdir().unwrap();
        write(&stick.path().join("b.pdf"), 18);
        write(&stick.path().join("wedding/1.jpg"), 5);
        write(&stick.path().join("wedding/2.jpg"), 7);
        write(&stick.path().join(".DS_Store"), 99);
        write(&stick.path().join(".ourstick-drive"), 64);
        assert_eq!(
            top_entries(stick.path()).unwrap(),
            vec![
                TopEntry {
                    name: "b.pdf".into(),
                    size: 18,
                    is_dir: false
                },
                TopEntry {
                    name: "wedding".into(),
                    size: 12,
                    is_dir: true
                },
            ]
        );
    }

    #[test]
    fn copies_keep_the_originals_and_never_overwrite() {
        let stick = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        write(&stick.path().join("notes.txt"), 3);
        write(&elsewhere.path().join("notes.txt"), 4);
        write(&elsewhere.path().join("photos/a.jpg"), 5);
        write(&elsewhere.path().join("photos/.DS_Store"), 50);
        let sources = [
            elsewhere.path().join("notes.txt"),
            elsewhere.path().join("photos"),
        ];

        let names = copy_onto(stick.path(), 100, &sources).unwrap();

        assert_eq!(names, vec!["notes (2).txt", "photos"]);
        assert_eq!(
            std::fs::read(stick.path().join("notes.txt")).unwrap().len(),
            3
        );
        assert_eq!(
            std::fs::read(stick.path().join("notes (2).txt"))
                .unwrap()
                .len(),
            4
        );
        assert!(stick.path().join("photos/a.jpg").exists());
        assert!(!stick.path().join("photos/.DS_Store").exists());
        assert!(elsewhere.path().join("notes.txt").exists(), "original kept");
        assert_eq!(free_name(stick.path(), "photos"), "photos (2)");
    }

    #[test]
    fn a_copy_that_does_not_fit_copies_nothing() {
        let stick = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        write(&stick.path().join("old.bin"), 60);
        write(&elsewhere.path().join("small.bin"), 10);
        write(&elsewhere.path().join("big.bin"), 40);
        let sources = [
            elsewhere.path().join("small.bin"),
            elsewhere.path().join("big.bin"),
        ];
        let result = copy_onto(stick.path(), 100, &sources);
        assert!(
            matches!(
                result,
                Err(DriveError::Full {
                    needed: 50,
                    free: 40
                })
            ),
            "{result:?}"
        );
        assert!(!stick.path().join("small.bin").exists());
    }

    #[test]
    fn the_stick_cannot_be_copied_onto_itself() {
        let stick = tempfile::tempdir().unwrap();
        write(&stick.path().join("a.txt"), 1);
        let result = copy_onto(stick.path(), 100, &[stick.path().join("a.txt")]);
        assert!(matches!(result, Err(DriveError::NotCopyable { .. })));
    }

    #[test]
    fn removing_takes_only_a_top_level_name() {
        let stick = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(&stick.path().join("a.txt"), 1);
        write(&stick.path().join("folder/b.txt"), 1);
        write(&outside.path().join("keep.txt"), 1);

        remove_from(stick.path(), "a.txt").unwrap();
        remove_from(stick.path(), "folder").unwrap();
        assert!(top_entries(stick.path()).unwrap().is_empty());
        for bad in ["", ".", "..", "../keep.txt", "folder/b.txt", "missing"] {
            assert!(
                matches!(
                    remove_from(stick.path(), bad),
                    Err(DriveError::NotOnStick { .. })
                ),
                "{bad}"
            );
        }
        assert!(outside.path().join("keep.txt").exists());
    }
}
