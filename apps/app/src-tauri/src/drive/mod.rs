//! The stick's folder shown as a drive (docs/gui-spec.md, "The stick is a
//! folder"). macOS: a sparse APFS disk image, attached so that Finder shows it
//! with an eject button. Linux: `~/OurStick`, bookmarked in the file
//! manager's sidebar. Eject detaches the image or removes the bookmark;
//! sharing goes on either way.

use std::path::{Path, PathBuf};

use anyhow::Result;
use ourstick_core::manifest::DRIVE_MARKER;

#[cfg(any(target_os = "linux", target_os = "macos", test))]
const NAME: &str = "OurStick";

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn home() -> Result<PathBuf> {
    use anyhow::Context;
    Ok(PathBuf::from(
        std::env::var_os("HOME").context("cannot find your home folder")?,
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

#[cfg(target_os = "linux")]
use linux::platform;
#[cfg(target_os = "macos")]
use macos::platform;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform {
    use super::*;
    use anyhow::bail;

    pub fn create(_size: u64) -> Result<PathBuf> {
        bail!("OurStick has no drive for this system yet")
    }
    pub fn mount(root: &Path) -> Result<PathBuf> {
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
