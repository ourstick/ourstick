//! Building a manifest by walking a folder on disk.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::encoding::validate_path;
use super::hashing::hash_file;
use super::ignore::is_ignored;
use super::{Manifest, ManifestError};
use crate::cipher::Cipher;

impl Manifest {
    /// Builds a manifest by walking `root` recursively. Every regular file is
    /// encrypted with `cipher` into a blob, and the entry records the BLAKE3 hash
    /// of that blob (the ciphertext, header included) and the plaintext size.
    /// The blobs themselves are hashed on the fly and not kept.
    ///
    /// - Entries are sorted by path, so the same folder always yields the same manifest.
    /// - Symlinks are not followed and are skipped, so a stick never exposes files
    ///   outside its root. Other non-regular files (sockets, devices) are skipped too.
    /// - Empty directories are not recorded; the manifest lists files only.
    /// - A file name that cannot be sent (not UTF-8, or containing `\`) is an error.
    ///
    /// - A file that changes while it is being encrypted is an error; try again later.
    ///
    /// This does blocking I/O. From async code, call it inside `spawn_blocking`.
    pub fn from_dir(root: impl AsRef<Path>, cipher: &dyn Cipher) -> Result<Self, ManifestError> {
        let root = root.as_ref();
        let mut entries = Vec::new();
        for path in regular_files(root)? {
            entries.push(hash_file(root, &path, cipher)?);
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self {
            entries,
            ..Self::default()
        })
    }
}

/// All regular files under `root`, in walk order. Symlinks are not followed and
/// not returned; other non-regular files are skipped, and so are system files
/// and folders ([`super::ignore`]).
pub(crate) fn regular_files(root: &Path) -> Result<Vec<PathBuf>, ManifestError> {
    let mut files = Vec::new();
    for_each_regular_file(root, &mut |path| {
        files.push(path.to_path_buf());
        Ok(())
    })?;
    Ok(files)
}

/// [`regular_files`], each given to `found` as the walk meets it, without
/// keeping them all.
pub(crate) fn for_each_regular_file(
    root: &Path,
    found: &mut dyn FnMut(&Path) -> Result<(), ManifestError>,
) -> Result<(), ManifestError> {
    let root_metadata = std::fs::metadata(root).map_err(|e| ManifestError::io(root, e))?;
    if !root_metadata.is_dir() {
        return Err(ManifestError::NotADirectory(root.to_path_buf()));
    }
    let walk = WalkDir::new(root).follow_links(false).into_iter();
    // Not the root itself: a stick may live in a folder of any name.
    let walk = walk.filter_entry(|item| {
        item.depth() == 0 || !item.file_name().to_str().is_some_and(is_ignored)
    });
    for item in walk {
        let item = item.map_err(|e| {
            let path = e.path().unwrap_or(root).to_path_buf();
            ManifestError::Io {
                path,
                source: e.into(),
            }
        })?;
        let file_type = item.file_type();
        if file_type.is_dir() {
            continue;
        }
        if !file_type.is_file() {
            tracing::debug!(path = %item.path().display(), "skipping non-regular file");
            continue;
        }
        found(item.path())?;
    }
    Ok(())
}

/// Turns `path` into a `/`-separated string relative to `root`.
pub(super) fn relative_path(root: &Path, path: &Path) -> Result<String, ManifestError> {
    let relative = path
        .strip_prefix(root)
        .expect("walkdir only yields paths under its root");
    // Built in one string: this runs for every file of a walk.
    let mut joined = String::with_capacity(relative.as_os_str().len());
    for component in relative.components() {
        let part = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| ManifestError::NonUtf8Path(path.to_path_buf()))?;
        if !joined.is_empty() {
            joined.push('/');
        }
        joined.push_str(part);
    }
    validate_path(&joined)?;
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::encrypt_blob;
    use crate::cipher::{ChaChaCipher, NoCipher, StickKey};
    use std::fs;
    use std::io::Cursor;
    use std::time::UNIX_EPOCH;

    #[test]
    fn from_dir_of_empty_folder_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("nothing-here")).unwrap();

        let manifest = Manifest::from_dir(dir.path(), &NoCipher).unwrap();
        assert_eq!(manifest, Manifest::empty());
    }

    #[test]
    fn from_dir_lists_nested_files_sorted_with_slash_paths() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("photos/2024")).unwrap();
        fs::write(dir.path().join("b.txt"), b"bravo").unwrap();
        fs::write(dir.path().join("a.txt"), b"alpha").unwrap();
        fs::write(dir.path().join("photos/2024/cat.jpg"), b"meow").unwrap();

        let manifest = Manifest::from_dir(dir.path(), &NoCipher).unwrap();
        let paths: Vec<&str> = manifest.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "b.txt", "photos/2024/cat.jpg"]);
    }

    fn chacha() -> ChaChaCipher {
        ChaChaCipher::new(&StickKey::from_bytes([5; 32]))
    }

    #[test]
    fn from_dir_records_plaintext_size_and_hash_of_ciphertext() {
        let dir = tempfile::tempdir().unwrap();
        let contents = vec![7u8; 200_000];
        fs::write(dir.path().join("big.bin"), &contents).unwrap();

        let ciphers: [&dyn Cipher; 2] = [&NoCipher, &chacha()];
        for cipher in ciphers {
            let mut blob = Vec::new();
            encrypt_blob(cipher, &mut Cursor::new(&contents), &mut blob).unwrap();

            let manifest = Manifest::from_dir(dir.path(), cipher).unwrap();
            let entry = &manifest.entries[0];
            assert_eq!(entry.size, contents.len() as u64);
            assert_eq!(entry.hash, *blake3::hash(&blob).as_bytes());
            assert_ne!(entry.hash, *blake3::hash(&contents).as_bytes());
            assert_eq!(manifest.total_size(), contents.len() as u64);
        }
    }

    #[test]
    fn from_dir_hash_depends_on_key() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), b"same content").unwrap();

        let first = Manifest::from_dir(dir.path(), &chacha()).unwrap();
        let other_key = ChaChaCipher::new(&StickKey::from_bytes([6; 32]));
        let second = Manifest::from_dir(dir.path(), &other_key).unwrap();
        assert_ne!(first.entries[0].hash, second.entries[0].hash);
    }

    #[test]
    fn from_dir_records_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        fs::write(&path, b"x").unwrap();
        let expected = fs::metadata(&path)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let manifest = Manifest::from_dir(dir.path(), &NoCipher).unwrap();
        assert_eq!(manifest.entries[0].mtime, expected);
    }

    #[test]
    fn from_dir_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["z", "m", "a", "q"] {
            fs::write(dir.path().join(name), name).unwrap();
        }

        let first = Manifest::from_dir(dir.path(), &chacha()).unwrap();
        let second = Manifest::from_dir(dir.path(), &chacha()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn from_dir_rejects_a_file_as_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        fs::write(&path, b"x").unwrap();

        let error = Manifest::from_dir(&path, &NoCipher).unwrap_err();
        assert!(matches!(error, ManifestError::NotADirectory(_)));
    }

    #[test]
    fn from_dir_of_missing_root_is_io_error() {
        let dir = tempfile::tempdir().unwrap();

        let error = Manifest::from_dir(dir.path().join("missing"), &NoCipher).unwrap_err();
        assert!(matches!(error, ManifestError::Io { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn from_dir_rejects_backslash_in_file_name() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a\\b.txt"), b"x").unwrap();

        let error = Manifest::from_dir(dir.path(), &NoCipher).unwrap_err();
        assert!(matches!(error, ManifestError::InvalidPath(_)));
    }

    #[cfg(unix)]
    #[test]
    fn from_dir_skips_symlinks() {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), b"do not share").unwrap();

        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("real.txt"), b"ok").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("link.txt"),
        )
        .unwrap();

        let manifest = Manifest::from_dir(dir.path(), &NoCipher).unwrap();
        let paths: Vec<&str> = manifest.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["real.txt"]);
    }
}
