//! What a stopped round leaves in the partial folder: a blob taken out of
//! the store, a file half decrypted. Nothing else writes there.

use std::path::Path;

use super::{PARTIAL_DIR, PARTIAL_MARK};

/// Removes the files a round before this one left in `partial` (this
/// download's, with its mark): no round is placing files while another
/// starts, and each leftover may be as big as a file of the stick. Only
/// files, never what is not one. Only tried.
pub(super) fn clear(partial: &Path) {
    let Ok(entries) = std::fs::read_dir(partial) else {
        return;
    };
    for entry in entries.flatten() {
        let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
        if is_file && entry.file_name() != PARTIAL_MARK {
            if let Err(error) = std::fs::remove_file(entry.path()) {
                tracing::debug!(%error, "a leftover of a stopped round stays");
            }
        }
    }
}

/// Removes the partial folder of `folder`, if it is this program's (it has
/// the mark): a stick forgotten, or stopped midway, leaves no hidden copies
/// behind. Never follows a link. Only tried. Blocking.
pub fn remove_partial_folder(folder: &Path) {
    let partial = folder.join(PARTIAL_DIR);
    let ours = std::fs::symlink_metadata(&partial).is_ok_and(|metadata| metadata.is_dir())
        && partial.join(PARTIAL_MARK).is_file();
    if ours {
        if let Err(error) = std::fs::remove_dir_all(&partial) {
            tracing::debug!(%error, "a forgotten stick's partial folder stays");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leftovers_go_and_the_folder_too_once_forgotten() {
        let folder = tempfile::tempdir().unwrap();
        let partial = super::super::prepare_partial_dir(folder.path()).unwrap();
        std::fs::write(partial.join("aa.blob"), b"blob").unwrap();
        std::fs::write(partial.join("aa.plain"), b"half").unwrap();
        std::fs::create_dir(partial.join("not-a-file")).unwrap();
        // The next round starts clean, its mark kept.
        let again = super::super::prepare_partial_dir(folder.path()).unwrap();
        assert!(!again.join("aa.blob").exists() && !again.join("aa.plain").exists());
        assert!(again.join(PARTIAL_MARK).is_file());
        assert!(again.join("not-a-file").is_dir());
        remove_partial_folder(folder.path());
        assert!(!partial.exists());
        // A folder without the mark is not this program's: left.
        std::fs::create_dir(&partial).unwrap();
        remove_partial_folder(folder.path());
        assert!(partial.exists());
    }
}
