//! The stick's folder shown as a drive (docs/gui-spec.md, "The stick is a
//! folder"). macOS: a sparse APFS disk image, attached so that Finder shows it
//! with an eject button. Linux: `~/OurStick`, bookmarked in the file
//! manager's sidebar. Windows: `%USERPROFILE%\OurStick`, shown as a drive
//! letter (`subst`). Eject detaches the image, removes the bookmark or takes
//! the letter away; sharing goes on either way.

use std::path::{Path, PathBuf};

use anyhow::Result;
use ourstick_core::manifest::DRIVE_MARKER;

#[cfg(any(target_os = "linux", target_os = "macos", windows, test))]
const NAME: &str = "OurStick";

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn home() -> Result<PathBuf> {
    use anyhow::Context;
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    Ok(PathBuf::from(
        std::env::var_os(variable).context("cannot find your home folder")?,
    ))
}

/// Whether the stick's folder is there and is the stick (its marker).
pub fn folder_is_there(root: &Path) -> bool {
    root.join(DRIVE_MARKER).is_file()
}

#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(any(windows, test))]
mod windows;

#[cfg(target_os = "linux")]
use linux::platform;
#[cfg(target_os = "macos")]
use macos::platform;
#[cfg(windows)]
use windows::platform;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
    use super::*;
    use anyhow::bail;

    pub fn create() -> Result<PathBuf> {
        bail!("OurStick has no drive for this system yet")
    }
    pub fn mount(root: &Path, _preferred: Option<char>) -> Result<PathBuf> {
        Ok(root.to_path_buf())
    }
    pub fn eject(_root: &Path) -> Result<()> {
        Ok(())
    }
    pub fn is_mounted(root: &Path) -> bool {
        folder_is_there(root)
    }
}

pub use platform::{create, eject, is_mounted, mount};

/// Where to measure what is left for the stick (ADR 0008): the disk that
/// really holds its files. On macOS that is the disk image's folder, not the
/// mounted volume (which tells the image's own size).
#[cfg(target_os = "macos")]
pub use platform::measure_at;
#[cfg(not(target_os = "macos"))]
pub fn measure_at(root: &Path) -> Result<PathBuf> {
    Ok(root.to_path_buf())
}

/// The drive letter that shows the stick now (Windows); `None` elsewhere.
#[cfg(windows)]
pub use platform::letter_of;
#[cfg(not(windows))]
pub fn letter_of(_root: &Path) -> Option<char> {
    None
}

/// Where the file manager should open the stick: its drive letter on
/// Windows, the folder elsewhere.
#[cfg(windows)]
pub use platform::shown_at;
#[cfg(not(windows))]
pub fn shown_at(root: &Path) -> PathBuf {
    root.to_path_buf()
}
