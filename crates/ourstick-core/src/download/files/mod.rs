//! Writing downloaded files: where they go, and how they get there safely.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use super::folder::may_replace;
use super::{io_error, DownloadError, PARTIAL_DIR};
use crate::blob::decrypt_blob;
use crate::cipher::Cipher;
use crate::manifest::{Entry, FileStamp, Manifest};
use crate::untrusted::escape;

/// Why one path component cannot be written safely on this reader, if it
/// cannot. Manifest paths are already free of `\`, NUL, `.`, `..` and empty
/// components (validated on decode); this adds what depends on the platform.
///
/// `windows` selects the Windows rules; it is a parameter so they are tested
/// everywhere.
fn component_problem(component: &str, windows: bool) -> Option<&'static str> {
    let mut components = Path::new(component).components();
    let single_normal = matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(name)), None) if name == component
    );
    if !single_normal {
        return Some("it is not a plain file or folder name here");
    }
    // `c:` would be a drive on Windows and move the path out of the output
    // folder; refused everywhere so a stick behaves the same on every reader.
    let bytes = component.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Some("it looks like a Windows drive");
    }
    if windows {
        if component.contains(['<', '>', ':', '"', '|', '?', '*'])
            || component.contains(char::is_control)
        {
            return Some("it contains a character Windows does not allow in names");
        }
        if component.ends_with('.') || component.ends_with(' ') {
            return Some("Windows drops a trailing dot or space from names");
        }
        let stem = component.split('.').next().unwrap_or(component).trim_end();
        let reserved = ["CON", "PRN", "AUX", "NUL"]
            .iter()
            .any(|name| stem.eq_ignore_ascii_case(name))
            || (stem.len() == 4
                && (stem[..3].eq_ignore_ascii_case("COM")
                    || stem[..3].eq_ignore_ascii_case("LPT"))
                && stem.as_bytes()[3].is_ascii_digit()
                && stem.as_bytes()[3] != b'0');
        if reserved {
            return Some("it is a reserved device name on Windows");
        }
    }
    None
}

/// Errors carry the path escaped: it comes from the owner.
fn unsafe_path(path: &str, reason: &'static str) -> DownloadError {
    DownloadError::UnsafePath {
        path: escape(path),
        reason,
    }
}

/// Checks every path of a manifest before anything is downloaded: each
/// component must be safe here, nothing may go into our partial directory (in
/// any letter case), and no two paths may land on the same file on a file
/// system that ignores letter case, or be both a file and a folder.
pub(super) fn check_manifest_paths(manifest: &Manifest) -> Result<(), DownloadError> {
    let mut by_folded_path: HashMap<String, &str> = HashMap::new();
    for entry in &manifest.entries {
        let path = entry.path.as_str();
        if path
            .split('/')
            .next()
            .is_some_and(|first| first.eq_ignore_ascii_case(PARTIAL_DIR))
        {
            return Err(unsafe_path(
                path,
                "that name is used by OurStick for unfinished files",
            ));
        }
        for component in path.split('/') {
            if let Some(reason) = component_problem(component, cfg!(windows)) {
                return Err(unsafe_path(path, reason));
            }
        }
        if let Some(earlier) = by_folded_path.insert(path.to_lowercase(), path) {
            return Err(DownloadError::PathCollision {
                first: escape(earlier),
                second: escape(path),
            });
        }
    }
    // A file may not also be a folder of another entry, in any letter case.
    for entry in &manifest.entries {
        let folded = entry.path.to_lowercase();
        let mut ancestor = folded.as_str();
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if let Some(file) = by_folded_path.get(parent) {
                return Err(DownloadError::PathCollision {
                    first: escape(file),
                    second: escape(&entry.path),
                });
            }
            ancestor = parent;
        }
    }
    Ok(())
}

/// Refuses a path that passes through a symbolic link inside `output_dir`.
fn check_no_symlinks(output_dir: &Path, relative: &str) -> Result<PathBuf, DownloadError> {
    let mut path = output_dir.to_path_buf();
    for component in relative.split('/') {
        path.push(component);
        if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(unsafe_path(
                relative,
                "it would pass through a symbolic link",
            ));
        }
    }
    Ok(path)
}

/// The final path of an entry whose path passed [`check_manifest_paths`].
/// Refuses to go through symbolic links that exist in the output folder now;
/// [`decrypt_into_place`] checks again right before the file is moved there.
pub(super) fn target_path(output_dir: &Path, relative: &str) -> Result<PathBuf, DownloadError> {
    check_no_symlinks(output_dir, relative)
}

/// Creates (or reuses) the partial directory, refusing one that is a symbolic
/// link, which would send unfinished files elsewhere.
pub(super) fn prepare_partial_dir(output_dir: &Path) -> Result<PathBuf, DownloadError> {
    let partial = output_dir.join(PARTIAL_DIR);
    let is_symlink = |path: &Path| {
        std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    };
    if is_symlink(&partial) {
        return Err(unsafe_path(PARTIAL_DIR, "it is a symbolic link"));
    }
    std::fs::create_dir_all(&partial).map_err(io_error(&partial))?;
    if is_symlink(&partial) {
        return Err(unsafe_path(PARTIAL_DIR, "it is a symbolic link"));
    }
    Ok(partial)
}

/// Decrypts `ciphertext` into `plaintext`, checks the size, sets the modification
/// time, and renames it to the entry's place under `output_dir`, checking for
/// symbolic links again right before. Removes `ciphertext` when done.
///
/// The file there is replaced only if it is still as planned (absent, or with
/// the stamp `replaceable`); otherwise it was changed meanwhile and is left
/// alone. Returns the stamp of the written file, or `None` if it was left.
pub(super) fn decrypt_into_place(
    cipher: &dyn Cipher,
    entry: &Entry,
    ciphertext: &Path,
    plaintext: &Path,
    output_dir: &Path,
    replaceable: Option<FileStamp>,
) -> Result<Option<FileStamp>, DownloadError> {
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
            path: escape(&entry.path),
            expected: entry.size,
            actual: written,
        });
    }
    output
        .set_modified(UNIX_EPOCH + Duration::from_secs(entry.mtime))
        .map_err(io_error(plaintext))?;
    output.sync_all().map_err(io_error(plaintext))?;
    drop(output);

    let placed = place(entry, plaintext, output_dir, replaceable)?;
    std::fs::remove_file(ciphertext).map_err(io_error(ciphertext))?;
    Ok(placed)
}

/// For a file whose blob another entry of this download already brought:
/// copies `source` (written by this download, with the stamp `source_stamp`)
/// into `plaintext` and places it as `entry`. Returns `None` if the source
/// changed meanwhile, so the caller fetches the blob instead; otherwise what
/// [`place`] returns.
pub(super) fn copy_into_place(
    entry: &Entry,
    source: &Path,
    source_stamp: FileStamp,
    plaintext: &Path,
    output_dir: &Path,
    replaceable: Option<FileStamp>,
) -> Result<Option<Option<FileStamp>>, DownloadError> {
    let unchanged =
        || std::fs::metadata(source).is_ok_and(|now| FileStamp::of(&now) == source_stamp);
    if !unchanged() {
        return Ok(None);
    }
    let copied = std::fs::copy(source, plaintext).map_err(io_error(plaintext))?;
    if copied != entry.size || !unchanged() {
        std::fs::remove_file(plaintext).map_err(io_error(plaintext))?;
        return Ok(None);
    }
    let output = File::options()
        .write(true)
        .open(plaintext)
        .map_err(io_error(plaintext))?;
    output
        .set_modified(UNIX_EPOCH + Duration::from_secs(entry.mtime))
        .map_err(io_error(plaintext))?;
    output.sync_all().map_err(io_error(plaintext))?;
    drop(output);
    Ok(Some(place(entry, plaintext, output_dir, replaceable)?))
}

/// Renames the finished `plaintext` to the entry's place under `output_dir`,
/// checking for symbolic links again right before. The file there is replaced
/// only if it is still as planned (absent, or with the stamp `replaceable`);
/// otherwise it was changed meanwhile, is left alone, and `plaintext` is
/// removed. Returns the stamp of the written file, or `None` if it was left.
fn place(
    entry: &Entry,
    plaintext: &Path,
    output_dir: &Path,
    replaceable: Option<FileStamp>,
) -> Result<Option<FileStamp>, DownloadError> {
    let target = check_no_symlinks(output_dir, &entry.path)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(io_error(parent))?;
    }
    // Again, now that the folders exist: nothing may have been swapped for a
    // symbolic link since the download started.
    let target = check_no_symlinks(output_dir, &entry.path)?;
    if !may_replace(&target, replaceable) {
        std::fs::remove_file(plaintext).map_err(io_error(plaintext))?;
        return Ok(None);
    }
    std::fs::rename(plaintext, &target).map_err(io_error(&target))?;
    let written = std::fs::metadata(&target).map_err(io_error(&target))?;
    Ok(Some(FileStamp::of(&written)))
}

#[cfg(test)]
mod tests;
