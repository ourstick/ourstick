//! Files that operating systems, file managers and OurStick itself put on a
//! drive or folder, which are never part of a stick: shared, they would only
//! clutter the reader's folder (and some, like `.DS_Store`, leak folder
//! details). The list is part of the protocol ("Owner-side rules").

/// Exact names, of a file or a folder, at any depth.
const NAMES: &[&str] = &[
    // macOS
    ".DS_Store",
    ".Spotlight-V100",
    ".fseventsd",
    ".Trashes",
    ".TemporaryItems",
    ".DocumentRevisions-V100",
    ".VolumeIcon.icns",
    ".apdisk",
    ".metadata_never_index",
    ".com.apple.timemachine.donotpresent",
    // Windows
    "Thumbs.db",
    "desktop.ini",
    "$RECYCLE.BIN",
    "System Volume Information",
    // Linux file managers
    ".directory",
    // OurStick: the drive marker, a reader's unfinished files, and the mark
    // of a received stick's folder
    super::DRIVE_MARKER,
    ".ourstick-partial",
    ".ourstick-received",
];

/// Name prefixes: AppleDouble files (`._photo.jpg`), per-user trash
/// folders (`.Trash-1000`), and every name OurStick writes beside its own
/// (`.ourstick-received.new`), as a receiver treats them (ADR 0010).
const PREFIXES: &[&str] = &["._", ".Trash-", ".ourstick-"];

/// Whether a file or folder with this name is left out of every stick.
/// The start of OurStick's own names, in any letter case.
const OURSTICK_PREFIX: &[u8] = b".ourstick-";

pub fn is_ignored(name: &str) -> bool {
    NAMES.contains(&name)
        || PREFIXES.iter().any(|prefix| name.starts_with(prefix))
        // OurStick's own names in any letter case, as a receiver takes them
        // (compared in place: this runs for every file of a walk).
        || name
            .as_bytes()
            .get(..OURSTICK_PREFIX.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(OURSTICK_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_files_are_ignored_and_ordinary_ones_are_not() {
        for name in [
            ".DS_Store",
            ".ourstick-received",
            "._photo.jpg",
            ".Spotlight-V100",
            ".Trash-1000",
            ".directory",
            "Thumbs.db",
            ".ourstick-drive",
            ".ourstick-partial",
            ".ourstick-received.new",
            ".OurStick-Partial",
        ] {
            assert!(is_ignored(name), "{name}");
        }
        for name in [
            "photo.jpg",
            ".bashrc",
            ".git",
            "_draft.txt",
            "DS_Store",
            ".Trash",
            "thumbs.db.bak",
            "notes.directory",
            ".ourstick",
        ] {
            assert!(!is_ignored(name), "{name}");
        }
    }

    #[test]
    fn a_manifest_leaves_system_files_out_at_any_depth() {
        use crate::cipher::NoCipher;
        use crate::manifest::Manifest;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for folder in ["photos", ".Spotlight-V100/Store", ".Trash-1000/files"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        for file in [
            "photos/a.jpg",
            "photos/.DS_Store",
            "photos/._a.jpg",
            ".DS_Store",
            ".Spotlight-V100/Store/index",
            ".Trash-1000/files/old.txt",
            ".ourstick-drive",
            "notes.txt",
        ] {
            std::fs::write(root.join(file), b"x").unwrap();
        }
        let manifest = Manifest::from_dir(root, &NoCipher).unwrap();
        let paths: Vec<&str> = manifest.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["notes.txt", "photos/a.jpg"]);
    }
}
