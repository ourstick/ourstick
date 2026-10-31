//! An older app stick's own folder (ADR 0028 §7): what is on its top level,
//! and what a copy of an older version left there. Nothing is copied onto a
//! stick any more. Blocking I/O throughout; call from `spawn_blocking`.

use std::path::{Path, PathBuf};

use crate::manifest::ignore::is_ignored;
use crate::share::{folder_size, ShareError};

/// Why the stick's top level could not be read. Messages are for users.
#[derive(Debug, thiserror::Error)]
pub enum DriveError {
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
            (
                folder_size(&crate::sender_files::LocalFiles, &entry.path())?,
                true,
            )
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

/// The hidden name an older version copied under first, then renamed.
const COPYING: &str = ".ourstick-copying-";

/// Removes what a copy of an older version, cut short, left behind (only a
/// copy's own names), never following a link. Blocking.
pub fn remove_leftovers(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let ours = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(COPYING));
        if !ours {
            continue;
        }
        let path = entry.path();
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {
                let _ = std::fs::remove_dir_all(&path);
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&path);
            }
            Err(_) => {}
        }
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
    fn a_free_name_is_numbered_as_file_managers_do() {
        let taken = ["notes.txt", "notes (2).txt", "v1.2"];
        let free = |name| free_name_where(name, |candidate| taken.contains(&candidate));
        assert_eq!(free("other.txt"), "other.txt");
        assert_eq!(free("notes.txt"), "notes (3).txt");
        assert_eq!(free("v1.2"), "v1 (2).2");
    }

    #[test]
    fn only_what_a_cut_copy_left_is_removed() {
        let stick = tempfile::tempdir().unwrap();
        write(&stick.path().join(".ourstick-copying-1/a.jpg"), 3);
        write(&stick.path().join(".ourstick-copying-2"), 3);
        write(&stick.path().join("photos/a.jpg"), 3);
        write(&stick.path().join(".ourstick-drive"), 64);
        remove_leftovers(stick.path());
        let mut left: Vec<String> = std::fs::read_dir(stick.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, [".ourstick-drive", "photos"]);
    }
}
