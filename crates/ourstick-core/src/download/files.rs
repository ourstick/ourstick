//! Writing downloaded files: where they go, and how they get there safely.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use super::{io_error, DownloadError, PARTIAL_DIR};
use crate::blob::decrypt_blob;
use crate::cipher::Cipher;
use crate::manifest::Entry;

/// The final path of an entry. Manifest paths are already validated (relative,
/// no `.` or `..`); this also refuses to go through symbolic links that exist in
/// the output folder, and paths inside our own partial directory.
pub(super) fn target_path(output_dir: &Path, relative: &str) -> Result<PathBuf, DownloadError> {
    let unsafe_path = || DownloadError::UnsafePath(relative.to_owned());
    if relative.split('/').next() == Some(PARTIAL_DIR) {
        return Err(unsafe_path());
    }
    let mut path = output_dir.to_path_buf();
    for component in relative.split('/') {
        path.push(component);
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(unsafe_path());
        }
    }
    Ok(path)
}

/// Decrypts `ciphertext` into `plaintext`, checks the size, sets the modification
/// time, and renames it to `target`. Removes `ciphertext` when done.
pub(super) fn decrypt_into_place(
    cipher: &dyn Cipher,
    entry: &Entry,
    ciphertext: &Path,
    plaintext: &Path,
    target: &Path,
) -> Result<(), DownloadError> {
    let source = File::open(ciphertext).map_err(io_error(ciphertext))?;
    let output = File::create_new(plaintext).map_err(io_error(plaintext))?;
    let mut writer = BufWriter::new(output);
    let written = decrypt_blob(cipher, &mut BufReader::new(source), &mut writer)?;
    writer.flush().map_err(io_error(plaintext))?;
    let output = writer
        .into_inner()
        .map_err(|error| io_error(plaintext)(error.into_error()))?;
    if written != entry.size {
        let _ = std::fs::remove_file(plaintext);
        return Err(DownloadError::SizeMismatch {
            path: entry.path.clone(),
            expected: entry.size,
            actual: written,
        });
    }
    output
        .set_modified(UNIX_EPOCH + Duration::from_secs(entry.mtime))
        .map_err(io_error(plaintext))?;
    output.sync_all().map_err(io_error(plaintext))?;
    drop(output);

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(io_error(parent))?;
    }
    std::fs::rename(plaintext, target).map_err(io_error(target))?;
    std::fs::remove_file(ciphertext).map_err(io_error(ciphertext))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_path_refuses_our_partial_dir_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            target_path(dir.path(), ".ourstick-partial/x"),
            Err(DownloadError::UnsafePath(_))
        ));
        assert_eq!(
            target_path(dir.path(), "a/b.txt").unwrap(),
            dir.path().join("a").join("b.txt")
        );
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
            assert!(matches!(
                target_path(dir.path(), "link/escape.txt"),
                Err(DownloadError::UnsafePath(_))
            ));
        }
    }
}
