//! Writing downloaded files: where they go, and how they get there safely.

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

/// The longest file or folder name the systems OurStick runs on take.
pub(crate) const MAX_NAME: usize = 255;

/// Why one path component cannot be written safely on this reader, if it
/// cannot. Manifest paths are already free of `\`, NUL, `.`, `..` and empty
/// components (validated on decode); this adds what depends on the platform.
/// The last check of every name written (ADR 0010).
///
/// `windows` selects the Windows rules; it is a parameter so they are tested
/// everywhere.
pub(crate) fn component_problem(component: &str, windows: bool) -> Option<&'static str> {
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
    // Longer than the system takes (255 UTF-16 units on Windows, 255 bytes
    // elsewhere): refused up front, not as a failed write half way.
    let length = if windows {
        component.encode_utf16().count()
    } else {
        component.len()
    };
    if length > MAX_NAME {
        return Some("the name is too long for this computer");
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
        if is_reserved_device(component) {
            return Some("it is a reserved device name on Windows");
        }
    }
    None
}

/// Whether Windows takes `name` for a device: `CON`, `PRN`, `AUX`, `NUL`,
/// `CONIN$`, `CONOUT$`, `COM0`–`COM9`, `LPT0`–`LPT9` (also with ¹ ² ³),
/// with or without an extension, ignoring case. `COM0` and `LPT0` are on
/// Microsoft's list though Windows lets most such names be: renamed all the
/// same, which costs a `_`. Compares whole characters, never byte offsets,
/// so a name in any script is safe here.
pub(crate) fn is_reserved_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        .iter()
        .any(|device| stem.eq_ignore_ascii_case(device))
    {
        return true;
    }
    let mut chars = stem.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let digit = chars.next();
    let numbered = prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT");
    numbered
        && chars.next().is_none()
        && digit.is_some_and(|digit| matches!(digit, '0'..='9' | '¹' | '²' | '³'))
}

/// Errors carry the path escaped: it comes from the owner.
fn unsafe_path(path: &str, reason: &'static str) -> DownloadError {
    DownloadError::UnsafePath {
        path: escape(path),
        reason,
    }
}

/// Checks every path of a manifest before anything is downloaded: each name
/// must be a plain file or folder name (decoding refuses the rest already;
/// checked again here, before any place is chosen). Names this computer cannot save, and names that would be one
/// file here, are not errors: they are saved under a changed name
/// (`saved_paths`, ADR 0010).
pub(crate) fn check_manifest_paths(manifest: &Manifest) -> Result<(), DownloadError> {
    for entry in &manifest.entries {
        // The same on every system (as the decoding's rules): Windows would
        // read `c:evil` as drive C:, which is a name to change, not refuse.
        let plain = entry
            .path
            .split('/')
            .all(|name| !matches!(name, "" | "." | "..") && !name.contains(['\\', '\0']));
        if !plain {
            return Err(unsafe_path(
                &entry.path,
                "it is not a plain file or folder name here",
            ));
        }
    }
    Ok(())
}

/// Refuses a path that passes through a symbolic link inside `output_dir`.
fn check_no_symlinks(output_dir: &Path, relative: &str) -> Result<PathBuf, DownloadError> {
    target_path_by(output_dir, relative, &|path, _| look(path))
}

/// What is at a place, for [`target_path_by`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    Link,
    /// Something not a link (or a place that cannot be looked at).
    There,
    /// Nothing: no place below it can be a link either.
    Missing,
}

/// What is at `path`, not following a link. Blocking.
pub(super) fn look(path: &Path) -> Place {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Place::Link,
        Ok(_) => Place::There,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Place::Missing,
        Err(_) => Place::There,
    }
}

/// [`target_path`], asking `look` what is at each place on the way (given
/// its full path and its path in the folder): a plan of many files asks
/// each folder once, and nothing below a place that is missing.
pub(super) fn target_path_by(
    output_dir: &Path,
    relative: &str,
    look: &dyn Fn(&Path, &str) -> Place,
) -> Result<PathBuf, DownloadError> {
    let mut path = output_dir.to_path_buf();
    let mut at = 0;
    let mut missing = false;
    for component in relative.split('/') {
        path.push(component);
        at += component.len();
        if missing {
            continue;
        }
        match look(&path, &relative[..at]) {
            Place::Link => {
                return Err(unsafe_path(
                    relative,
                    "it would pass through a symbolic link",
                ))
            }
            Place::Missing => missing = true,
            Place::There => {}
        }
        at += 1;
    }
    Ok(path)
}

/// The final path of an entry whose path passed [`check_manifest_paths`].
/// Refuses to go through symbolic links that exist in the output folder now;
/// [`decrypt_into_place`] checks again right before the file is moved there.
pub(super) fn target_path(output_dir: &Path, relative: &str) -> Result<PathBuf, DownloadError> {
    check_no_symlinks(output_dir, relative)
}

/// The mark a download leaves in the partial folder it made. A folder made
/// again by anything else (iroh makes a missing folder when it exports a
/// blob) lacks it: that is an unplugged disk's folder made again on another
/// disk, and nothing more is written there.
pub(super) const PARTIAL_MARK: &str = ".ourstick-here";

/// Whether `partial` is still the partial folder this download made; else
/// `NotFound`, as for a folder that is gone.
pub(super) fn partial_there(partial: &Path) -> Result<(), DownloadError> {
    if partial.join(PARTIAL_MARK).is_file() {
        Ok(())
    } else {
        Err(io_error(partial)(std::io::Error::from(
            std::io::ErrorKind::NotFound,
        )))
    }
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
    // Inside the folder as it is: a folder that is gone (an unplugged disk's
    // empty mount point) is never made again on another disk.
    match std::fs::create_dir(&partial) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(io_error(&partial)(error)),
    }
    if is_symlink(&partial) {
        return Err(unsafe_path(PARTIAL_DIR, "it is a symbolic link"));
    }
    super::receive::write_hidden(&partial.join(PARTIAL_MARK), b"").map_err(io_error(&partial))?;
    hide_folder(&partial);
    Ok(partial)
}

/// Hides `folder` on Windows (a dot hides it elsewhere), so the files coming
/// in are not seen, or deleted, half way: with Windows' own `attrib`, which
/// needs no unsafe code. Only tried.
fn hide_folder(folder: &Path) {
    #[cfg(windows)]
    {
        let hidden = std::process::Command::new("attrib")
            .arg("+h")
            .arg(folder)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if !hidden.is_ok_and(|status| status.success()) {
            tracing::debug!("the partial folder stays visible");
        }
    }
    #[cfg(not(windows))]
    let _ = folder;
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
    from_internet::mark(plaintext);
    set_mtime(&output, entry.mtime);
    output.sync_all().map_err(io_error(plaintext))?;
    drop(output);

    let placed = place(entry, plaintext, output_dir, replaceable)?;
    std::fs::remove_file(ciphertext).map_err(io_error(ciphertext))?;
    Ok(placed)
}

/// The modification time the sender gave, if this system can hold it. The
/// time comes from a peer: one past what `SystemTime` holds would panic, and
/// one the file system refuses (far in the future, before 1970 on some) only
/// leaves the file with the time it was written.
fn set_mtime(output: &File, mtime: u64) {
    let Some(time) = UNIX_EPOCH.checked_add(Duration::from_secs(mtime)) else {
        tracing::debug!(mtime, "a received file's time is out of range; not set");
        return;
    };
    if let Err(error) = output.set_modified(time) {
        tracing::debug!(%error, mtime, "a received file's time could not be set");
    }
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
    from_internet::mark(plaintext);
    set_mtime(&output, entry.mtime);
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
    // The folder must still be there (with its unfinished files): a disk
    // unplugged meanwhile is not filled in its place.
    partial_there(&output_dir.join(PARTIAL_DIR))?;
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

mod from_internet;

#[cfg(test)]
mod tests;
