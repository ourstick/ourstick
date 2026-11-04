//! The data folder, private to the user.

use std::path::Path;

use super::DeviceError;

/// Makes the data folder private to the user, if made here: one that was
/// there (`OURSTICK_DATA_DIR` may name any) keeps its mode, with a warning.
pub(super) fn create_private_dir(dir: &Path) -> Result<(), DeviceError> {
    #[cfg(unix)]
    let existed = dir.is_dir();
    std::fs::create_dir_all(dir).map_err(super::io(dir))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir)
            .map_err(super::io(dir))?
            .permissions()
            .mode();
        if !existed {
            let private = std::fs::Permissions::from_mode(0o700);
            std::fs::set_permissions(dir, private).map_err(super::io(dir))?;
        } else if mode & 0o077 != 0 {
            tracing::warn!(mode = mode & 0o777, "the data folder can be read by others");
        }
    }
    Ok(())
}
