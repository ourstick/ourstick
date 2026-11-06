//! A received file is marked as one from the internet (ADR 0032), as a
//! browser marks a download: Windows then has SmartScreen and Office's
//! Protected View look at it before it runs or opens, and macOS has
//! Gatekeeper. Best effort: a disk that cannot hold the mark (FAT, exFAT)
//! keeps the file without it.

use std::path::Path;

/// Marks `file`, before it is moved into place. On Windows the mark is a
/// stream of the file, and writing it changes the file's time: so this
/// comes before the time is set.
pub(super) fn mark(file: &Path) {
    #[cfg(windows)]
    windows::mark(file);
    #[cfg(target_os = "macos")]
    macos::mark(file);
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = file;
}

/// Marks a folder a reader made, as Archive Utility marks every folder it
/// unpacks (ADR 0032, amendment 1): on macOS the quarantine, so that an app
/// bundle inside a stick is in quarantine itself, not only its files.
/// Windows has no mark for a folder, Linux and Android none at all. Not
/// through a link: a folder swapped for one meanwhile is refused after.
pub(crate) fn mark_folder(folder: &Path) {
    #[cfg(target_os = "macos")]
    macos::mark_folder(folder);
    #[cfg(not(target_os = "macos"))]
    let _ = folder;
}

/// Gives the folder `to`, made by a copy of the folder `from` onto another
/// disk, the mark `from` has (macOS; a copied file keeps its own): a move
/// adds no mark and takes none away.
pub(crate) fn copy_folder_mark(from: &Path, to: &Path) {
    #[cfg(target_os = "macos")]
    macos::copy_folder_mark(from, to);
    #[cfg(not(target_os = "macos"))]
    let _ = (from, to);
}

/// Makes the folders from `output_dir` down to `folder` that are not there
/// yet, each marked as from the internet as soon as it is made (ADR 0032,
/// amendment 1). One made meanwhile by another file of the download is
/// that one's to mark.
pub(super) fn make_folders(output_dir: &Path, folder: &Path) -> std::io::Result<()> {
    let Ok(below) = folder.strip_prefix(output_dir) else {
        return std::fs::create_dir_all(folder);
    };
    let mut made = output_dir.to_path_buf();
    for part in below.components() {
        made.push(part);
        match std::fs::create_dir(&made) {
            Ok(()) => mark_folder(&made),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && made.is_dir() => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(windows)]
mod windows {
    use std::path::Path;

    /// The Mark of the Web: zone 3, the internet.
    pub(super) const ZONE_IDENTIFIER: &str = "[ZoneTransfer]\r\nZoneId=3\r\n";

    pub(super) fn mark(file: &Path) {
        let mut stream = file.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        if let Err(error) = std::fs::write(&stream, ZONE_IDENTIFIER) {
            tracing::debug!(%error, "a received file could not be marked as from the internet");
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// The quarantine as Safari writes it: flags (a download), the time in
    /// hex, the agent.
    pub(super) fn value(now: SystemTime) -> String {
        let seconds = now
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_secs());
        format!("0081;{seconds:x};OurStick;")
    }

    const QUARANTINE: &str = "com.apple.quarantine";

    /// Set as Safari sets it, through the system call itself (rustix, no
    /// process per file: a stick of millions of files would spend hours
    /// starting them).
    pub(super) fn mark(file: &Path) {
        let set = rustix::fs::setxattr(
            file,
            QUARANTINE,
            value(SystemTime::now()).as_bytes(),
            rustix::fs::XattrFlags::empty(),
        );
        if let Err(error) = set {
            tracing::debug!(%error, "a received file could not be put in quarantine");
        }
    }

    /// As [`mark`], on the folder itself, never on what a link points to.
    pub(super) fn mark_folder(folder: &Path) {
        let set = rustix::fs::lsetxattr(
            folder,
            QUARANTINE,
            value(SystemTime::now()).as_bytes(),
            rustix::fs::XattrFlags::empty(),
        );
        if let Err(error) = set {
            tracing::debug!(%error, "a received folder could not be put in quarantine");
        }
    }

    pub(super) fn copy_folder_mark(from: &Path, to: &Path) {
        // A quarantine is a few dozen bytes: flags, time, agent, an id.
        let mut found = [0u8; 512];
        let Ok(length) = rustix::fs::lgetxattr(from, QUARANTINE, &mut found) else {
            return;
        };
        let set = rustix::fs::lsetxattr(
            to,
            QUARANTINE,
            &found[..length],
            rustix::fs::XattrFlags::empty(),
        );
        if let Err(error) = set {
            tracing::debug!(%error, "a moved folder could not keep its quarantine");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marked_file_keeps_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("setup.exe");
        std::fs::write(&file, b"MZ").unwrap();
        mark(&file);
        assert_eq!(std::fs::read(&file).unwrap(), b"MZ");
        // A file that is not there is no error.
        mark(&dir.path().join("gone"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_has_the_mark_of_the_web() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("setup.exe");
        std::fs::write(&file, b"MZ").unwrap();
        mark(&file);
        let mut stream = file.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        assert_eq!(
            std::fs::read_to_string(stream).unwrap(),
            windows::ZONE_IDENTIFIER
        );
    }

    #[test]
    fn a_marked_folder_is_still_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Tool.app");
        std::fs::create_dir(&folder).unwrap();
        mark_folder(&folder);
        let moved = dir.path().join("moved.app");
        std::fs::create_dir(&moved).unwrap();
        copy_folder_mark(&folder, &moved);
        assert!(folder.is_dir() && moved.is_dir());
        // A folder that is not there is no error.
        mark_folder(&dir.path().join("gone"));
        copy_folder_mark(&dir.path().join("gone"), &moved);
    }

    /// The quarantine of `path` as `xattr` reads it ("" without one).
    #[cfg(target_os = "macos")]
    fn quarantine(path: &Path) -> String {
        let read = std::process::Command::new("/usr/bin/xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&read.stdout).trim().to_owned()
    }

    #[test]
    fn folders_are_made_one_by_one_and_only_new_ones_are_marked() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        std::fs::create_dir(out.join("kept")).unwrap();
        let deep = out.join("kept/Tool.app/Contents/MacOS");
        make_folders(out, &deep).unwrap();
        assert!(deep.is_dir());
        // Again: nothing to make.
        make_folders(out, &deep).unwrap();
        // A file in a folder's place is an error, as for create_dir_all.
        std::fs::write(out.join("file"), b"x").unwrap();
        assert!(make_folders(out, &out.join("file/below")).is_err());
        #[cfg(target_os = "macos")]
        {
            assert!(quarantine(&out.join("kept/Tool.app")).starts_with("0081;"));
            assert!(quarantine(&deep).starts_with("0081;"));
            assert_eq!(quarantine(&out.join("kept")), "");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_puts_folders_in_quarantine_and_a_move_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Tool.app");
        std::fs::create_dir(&bundle).unwrap();
        mark_folder(&bundle);
        assert!(quarantine(&bundle).starts_with("0081;"));
        let moved = dir.path().join("moved.app");
        std::fs::create_dir(&moved).unwrap();
        copy_folder_mark(&bundle, &moved);
        assert_eq!(quarantine(&moved), quarantine(&bundle));
        // A folder without the mark gives none.
        let plain = dir.path().join("plain");
        let copy = dir.path().join("copy");
        std::fs::create_dir(&plain).unwrap();
        std::fs::create_dir(&copy).unwrap();
        copy_folder_mark(&plain, &copy);
        assert_eq!(quarantine(&copy), "");
        // Never through a link: the folder it points to stays unmarked.
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&plain, &link).unwrap();
        mark_folder(&link);
        assert_eq!(quarantine(&plain), "");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_has_the_quarantine() {
        use std::time::{Duration, UNIX_EPOCH};
        assert_eq!(
            macos::value(UNIX_EPOCH + Duration::from_secs(0x6000_0000)),
            "0081;60000000;OurStick;"
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tool");
        std::fs::write(&file, b"#!").unwrap();
        mark(&file);
        let read = std::process::Command::new("/usr/bin/xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(&file)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&read.stdout).starts_with("0081;"));
    }
}
