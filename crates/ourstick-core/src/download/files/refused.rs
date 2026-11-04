//! A place the folder's disk refuses for one file.

use std::path::Path;

use crate::manifest::FileStamp;

/// A file whose own place the folder's disk refuses (a name a FAT disk or
/// a Windows share cannot hold, a folder of it the user closed) is left
/// out, as one changed there is: the others come, and the round is not
/// failed for it every time. Any other error stays one.
pub(super) fn its_place_refused(
    plaintext: &Path,
    error: std::io::Error,
) -> Result<Option<FileStamp>, std::io::Error> {
    // EILSEQ: a name the file system cannot encode (Linux and Android 84,
    // macOS 92).
    let unencodable = matches!(
        (error.raw_os_error(), cfg!(target_os = "macos")),
        (Some(84), false) | (Some(92), true)
    );
    let refused = unencodable
        || matches!(
            error.kind(),
            std::io::ErrorKind::InvalidInput
                | std::io::ErrorKind::InvalidFilename
                | std::io::ErrorKind::PermissionDenied
                | std::io::ErrorKind::NotADirectory
        );
    if !refused {
        return Err(error);
    }
    tracing::warn!(%error, "a file left out: its place here cannot be used");
    let _ = std::fs::remove_file(plaintext);
    Ok(None)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_closed_folder_leaves_one_file_out_and_other_errors_stay() {
        let dir = tempfile::tempdir().unwrap();
        let plaintext = dir.path().join("x.plain");
        std::fs::write(&plaintext, b"data").unwrap();
        let closed = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(its_place_refused(&plaintext, closed), Ok(None)));
        assert!(!plaintext.exists(), "its unfinished copy goes");
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        assert!(its_place_refused(&plaintext, full).is_err());
        let unencodable =
            std::io::Error::from_raw_os_error(if cfg!(target_os = "macos") { 92 } else { 84 });
        assert!(matches!(
            its_place_refused(&plaintext, unencodable),
            Ok(None)
        ));
    }
}
