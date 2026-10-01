//! Emptying a removed app stick's folder: only what is the stick's.

use std::path::{Path, PathBuf};

use super::LifeError;
use crate::linked::disk_of;
use crate::manifest::DRIVE_MARKER;
use crate::places::comparable;

/// Whether `root` is a place no stick's folder can be: the top of a disk,
/// the home folder, or one holding or inside OurStick's data folder. A
/// corrupt row naming one is refused, marker or not.
fn never_a_stick(root: &Path, data_dir: &Path) -> bool {
    let root = comparable(root);
    let data = comparable(data_dir);
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(|home| comparable(Path::new(&home)));
    root.parent().is_none()
        || Some(&root) == home.as_ref()
        || data.starts_with(&root)
        || root.starts_with(&data)
}

/// Deletes what is in a drive's folder, then its marker, then the folder if
/// it is empty: never following a link, never crossing into another disk
/// mounted inside (left in place). What cannot be deleted is skipped and
/// told, and the marker stays, so the removal is tried again and still
/// finds it. (Linux: a bind mount of the same disk inside cannot be told
/// apart from a folder.)
pub(super) fn empty_drive(root: &Path, data_dir: &Path) -> Result<(), LifeError> {
    if never_a_stick(root, data_dir) {
        return Err(LifeError::Drive(format!(
            "{} cannot be a stick's folder; nothing was deleted",
            root.display()
        )));
    }
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| LifeError::Io { path, source }
    };
    let disk = disk_of(&std::fs::metadata(root).map_err(io(root))?);
    let mut failed: Vec<(PathBuf, std::io::Error)> = Vec::new();
    for entry in std::fs::read_dir(root).map_err(io(root))? {
        let entry = entry.map_err(io(root))?;
        if entry.file_name() != DRIVE_MARKER {
            delete_tree(&entry.path(), disk, &mut failed);
        }
    }
    if let Some((path, source)) = failed.into_iter().next() {
        return Err(LifeError::Io { path, source });
    }
    let marker = root.join(DRIVE_MARKER);
    std::fs::remove_file(&marker).map_err(io(&marker))?;
    // Not empty: something on another disk was left; that is the user's.
    let _ = std::fs::remove_dir(root);
    Ok(())
}

/// Whether `metadata` is a link to a folder that Windows deletes as a
/// folder (a directory symbolic link or a junction), never to be entered.
#[cfg(windows)]
fn is_folder_link(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const DIRECTORY: u32 = 0x10;
    const REPARSE_POINT: u32 = 0x400;
    let attributes = metadata.file_attributes();
    attributes & REPARSE_POINT != 0 && attributes & DIRECTORY != 0
}

#[cfg(not(windows))]
fn is_folder_link(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn delete_tree(path: &Path, disk: Option<u64>, failed: &mut Vec<(PathBuf, std::io::Error)>) {
    let mut note = |result: std::io::Result<()>| {
        if let Err(source) = result {
            failed.push((path.to_path_buf(), source));
        }
    };
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => return note(Err(error)),
    };
    if is_folder_link(&metadata) {
        // The link goes; what it points to is not the stick's.
        return note(std::fs::remove_dir(path));
    }
    if !metadata.is_dir() {
        return note(std::fs::remove_file(path));
    }
    if disk.is_some() && disk_of(&metadata) != disk {
        // Another disk mounted inside the stick: not the stick's.
        return;
    }
    match std::fs::read_dir(path) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) => delete_tree(&entry.path(), disk, failed),
                    Err(error) => failed.push((path.to_path_buf(), error)),
                }
            }
        }
        Err(error) => return failed.push((path.to_path_buf(), error)),
    }
    // Not empty if something inside was on another disk.
    let _ = std::fs::remove_dir(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disk_top_the_home_or_the_data_folder_is_never_emptied() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(data.join("cache")).unwrap();
        for root in [Path::new("/"), dir.path(), &data, &data.join("cache")] {
            assert!(never_a_stick(root, &data), "{}", root.display());
            assert!(matches!(empty_drive(root, &data), Err(LifeError::Drive(_))));
        }
        if let Some(home) = std::env::var_os("HOME") {
            assert!(never_a_stick(Path::new(&home), &data));
        }
        assert!(!never_a_stick(&dir.path().join("OurStick/Family"), &data));
        assert!(data.join("cache").exists());
    }

    #[cfg(unix)]
    #[test]
    fn links_inside_are_deleted_not_followed_and_what_fails_is_told() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let outside = dir.path().join("outside");
        let root = dir.path().join("OurStick/Family");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
        std::fs::create_dir_all(root.join("locked")).unwrap();
        std::fs::write(root.join("locked/file"), b"x").unwrap();
        std::fs::write(root.join(DRIVE_MARKER), b"id").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("to-outside")).unwrap();
        // A folder that cannot be emptied (unless running as root).
        std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o500))
            .unwrap();
        let result = empty_drive(&root, &data);
        std::fs::set_permissions(root.join("locked"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        assert!(
            outside.join("keep.txt").exists(),
            "a link is never followed"
        );
        assert!(!root.join("to-outside").exists());
        if result.is_err() {
            // Told, and the marker is kept for the next try.
            assert!(root.join(DRIVE_MARKER).exists());
        }
        empty_drive(&root, &data).unwrap();
        assert!(!root.exists());
    }
}
