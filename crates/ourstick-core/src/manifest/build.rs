//! Building a manifest by walking a folder on disk.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use walkdir::WalkDir;

use super::encoding::validate_path;
use super::ignore::is_ignored;
use super::{Entry, Manifest, ManifestError};
use crate::blob::encrypt_blob;
use crate::cipher::{Cipher, CipherError};

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
            entries.push(hash_file(root, &path, cipher, None)?.0);
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self {
            entries,
            ..Self::default()
        })
    }

    /// Like [`Manifest::from_dir`], but also keeps every blob: each one is
    /// written to `blob_dir/<hex hash>.blob` and the paths are returned.
    ///
    /// A blob is first written under a temporary name and renamed only after
    /// `encrypt_blob` confirmed the file did not change while it was read
    /// (ADR 0003), so a file in `blob_dir` named after a hash always holds
    /// exactly that blob. On error, the temporary file is removed.
    pub fn from_dir_keeping_blobs(
        root: impl AsRef<Path>,
        cipher: &dyn Cipher,
        blob_dir: &Path,
    ) -> Result<(Self, Vec<PathBuf>), ManifestError> {
        let root = root.as_ref();
        let mut entries = Vec::new();
        let mut blobs = Vec::new();
        for path in regular_files(root)? {
            let (entry, blob) = hash_file(root, &path, cipher, Some(blob_dir))?;
            entries.push(entry);
            blobs.extend(blob);
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok((
            Self {
                entries,
                ..Self::default()
            },
            blobs,
        ))
    }
}

/// All regular files under `root`, in walk order. Symlinks are not followed and
/// not returned; other non-regular files are skipped, and so are system files
/// and folders ([`super::ignore`]).
pub(crate) fn regular_files(root: &Path) -> Result<Vec<PathBuf>, ManifestError> {
    let root_metadata = std::fs::metadata(root).map_err(|e| ManifestError::io(root, e))?;
    if !root_metadata.is_dir() {
        return Err(ManifestError::NotADirectory(root.to_path_buf()));
    }
    let mut files = Vec::new();
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
        files.push(item.into_path());
    }
    Ok(files)
}

/// Writes the blob both to the hasher and, if kept, to a file.
struct HashAndKeep<'a> {
    hasher: &'a mut blake3::Hasher,
    file: Option<BufWriter<File>>,
}

impl Write for HashAndKeep<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Some(file) = &mut self.file {
            file.write_all(bytes)?;
        }
        self.hasher.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut self.file {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

/// Encrypts one file, hashes the resulting blob and builds the entry. The size is
/// the number of plaintext bytes actually encrypted, so it always matches the hash.
/// With `blob_dir`, the blob is also kept as `blob_dir/<hex hash>.blob`.
pub(crate) fn hash_file(
    root: &Path,
    path: &Path,
    cipher: &dyn Cipher,
    blob_dir: Option<&Path>,
) -> Result<(Entry, Option<PathBuf>), ManifestError> {
    let file = File::open(path).map_err(|e| ManifestError::io(path, e))?;
    let metadata = file.metadata().map_err(|e| ManifestError::io(path, e))?;

    let temporary = blob_dir.map(|dir| dir.join(format!(".{}.partial", unique_suffix())));
    let kept_file = match &temporary {
        Some(temporary) => Some(BufWriter::new(
            File::create_new(temporary).map_err(|e| ManifestError::io(temporary, e))?,
        )),
        None => None,
    };
    let mut blob_hasher = blake3::Hasher::new();
    let mut sink = HashAndKeep {
        hasher: &mut blob_hasher,
        file: kept_file,
    };
    let encrypted = encrypt_blob(cipher, &mut BufReader::new(file), &mut sink)
        .and_then(|info| {
            if let Some(file) = sink.file.take() {
                let file = file.into_inner().map_err(|error| error.into_error())?;
                file.sync_all()?;
            }
            Ok(info)
        })
        .map_err(|error| match error {
            CipherError::Io(source) => ManifestError::io(path, source),
            source => ManifestError::Blob {
                path: path.to_path_buf(),
                source,
            },
        });
    let info = match encrypted {
        Ok(info) => info,
        Err(error) => {
            if let Some(temporary) = &temporary {
                let _ = std::fs::remove_file(temporary);
            }
            return Err(error);
        }
    };
    let hash = *blob_hasher.finalize().as_bytes();
    let kept = match (temporary, blob_dir) {
        (Some(temporary), Some(dir)) => {
            let target = dir.join(format!("{}.blob", data_encoding::HEXLOWER.encode(&hash)));
            std::fs::rename(&temporary, &target).map_err(|e| ManifestError::io(&target, e))?;
            Some(target)
        }
        _ => None,
    };

    let mtime = metadata
        .modified()
        .map_err(|e| ManifestError::io(path, e))?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let entry = Entry {
        path: relative_path(root, path)?,
        size: info.plaintext_len,
        mtime,
        hash,
    };
    Ok((entry, kept))
}

/// A random suffix for temporary file names.
fn unique_suffix() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    data_encoding::HEXLOWER.encode(&bytes)
}

/// Turns `path` into a `/`-separated string relative to `root`.
pub(super) fn relative_path(root: &Path, path: &Path) -> Result<String, ManifestError> {
    let relative = path
        .strip_prefix(root)
        .expect("walkdir only yields paths under its root");
    let parts = relative
        .components()
        .map(|c| {
            c.as_os_str()
                .to_str()
                .ok_or_else(|| ManifestError::NonUtf8Path(path.to_path_buf()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let joined = parts.join("/");
    validate_path(&joined)?;
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::{ChaChaCipher, NoCipher, StickKey};
    use std::fs;
    use std::io::Cursor;

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
    fn keeping_blobs_writes_each_blob_under_its_hash() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("a.txt"), b"alpha").unwrap();
        fs::write(dir.path().join("sub/b.bin"), vec![3u8; 70_000]).unwrap();

        let (manifest, kept) =
            Manifest::from_dir_keeping_blobs(dir.path(), &chacha(), blobs.path()).unwrap();
        assert_eq!(manifest, Manifest::from_dir(dir.path(), &chacha()).unwrap());
        assert_eq!(kept.len(), 2);
        for entry in &manifest.entries {
            let name = format!("{}.blob", data_encoding::HEXLOWER.encode(&entry.hash));
            let bytes = fs::read(blobs.path().join(name)).unwrap();
            assert_eq!(*blake3::hash(&bytes).as_bytes(), entry.hash);
        }
        // No temporary files left behind.
        assert_eq!(fs::read_dir(blobs.path()).unwrap().count(), 2);
    }

    #[test]
    fn keeping_blobs_fails_cleanly_when_blob_dir_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"alpha").unwrap();
        let missing = dir.path().join("no-such-dir");
        assert!(matches!(
            Manifest::from_dir_keeping_blobs(dir.path(), &chacha(), &missing),
            Err(ManifestError::Io { .. })
        ));
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
