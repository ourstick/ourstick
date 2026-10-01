//! Each stick's folder shown as a drive (docs/gui-spec.md, "The stick is a
//! folder"; ADR 0009 §3). macOS: a sparse APFS disk image per stick,
//! attached so that Finder shows it with an eject button. Linux: a folder
//! in `~/OurStick`, bookmarked under the stick's name. Windows: a folder in
//! `%USERPROFILE%\OurStick`, shown as a drive letter (`subst`). Eject
//! detaches the image, removes the bookmark or takes the letter away;
//! sharing goes on either way.

use std::path::{Path, PathBuf};

use anyhow::Result;
use ourstick_core::manifest::DRIVE_MARKER;
use ourstick_core::OwnedStick;

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

    pub fn sticks_home() -> Result<PathBuf> {
        bail!("OurStick has no drive for this system yet")
    }
    pub fn create(_folder: &str, _name: &str) -> Result<(PathBuf, Option<PathBuf>)> {
        bail!("OurStick has no drive for this system yet")
    }
    pub fn mount(stick: &OwnedStick) -> Result<PathBuf> {
        Ok(stick.root_path.clone())
    }
    pub fn eject(_stick: &OwnedStick) -> Result<()> {
        Ok(())
    }
    pub fn is_mounted(stick: &OwnedStick) -> bool {
        folder_is_there(&stick.root_path)
    }
}

pub use platform::{create, eject, is_mounted, mount, sticks_home};

/// The first stick of a version before ADR 0009 is `~/OurStick` itself
/// (Linux, Windows): it moves into it as `~/OurStick/<folder>` before a
/// second stick is made, so that no stick lies inside another (ADR 0009
/// §4). Its share is stopped first; its bookmark or letter goes and comes
/// back. Through `~/.OurStick-moving`; [`finish_move`] completes a move
/// stopped halfway. Returns where it is now (unchanged if it needed no
/// move). macOS: images do not nest, nothing moves.
#[cfg(not(target_os = "macos"))]
pub fn move_first(stick: &OwnedStick, folder: &str) -> Result<PathBuf> {
    use anyhow::Context;
    let home = sticks_home()?;
    if stick.root_path != home {
        return Ok(stick.root_path.clone());
    }
    eject(stick)?;
    let moving = moving_place(&home);
    std::fs::rename(&home, &moving).with_context(|| {
        format!(
            "cannot move {}: if a file in it is open, close it and try again",
            home.display()
        )
    })?;
    finish_move(&home, &moving, folder)
}

#[cfg(target_os = "macos")]
pub fn move_first(stick: &OwnedStick, _folder: &str) -> Result<PathBuf> {
    Ok(stick.root_path.clone())
}

/// Where the first stick waits while it moves.
#[cfg(not(target_os = "macos"))]
fn moving_place(home: &Path) -> PathBuf {
    home.with_file_name(".OurStick-moving")
}

/// The second half of a move: `~/OurStick` made anew, the stick moved in.
#[cfg(not(target_os = "macos"))]
fn finish_move(home: &Path, moving: &Path, folder: &str) -> Result<PathBuf> {
    use anyhow::Context;
    if !home.exists() {
        std::fs::create_dir(home).with_context(|| format!("cannot create {}", home.display()))?;
    }
    let root = home.join(folder);
    std::fs::rename(moving, &root)
        .with_context(|| format!("cannot move the stick into {}", root.display()))?;
    Ok(root)
}

/// At start: if `stick` is not where it is recorded because a move stopped
/// halfway (it is at `~/.OurStick-moving`, or already in `~/OurStick`), the
/// move is finished; returns where it is now, to record. `None` if it is
/// where it is recorded, or nowhere to be found (ejected, say).
#[cfg(not(target_os = "macos"))]
pub fn find_moved(stick: &OwnedStick, folder: &str) -> Result<Option<PathBuf>> {
    use ourstick_core::share::drive_marked_by;
    if drive_marked_by(&stick.root_path, &stick.stick_id) {
        return Ok(None);
    }
    let home = sticks_home()?;
    let moving = moving_place(&home);
    if drive_marked_by(&moving, &stick.stick_id) {
        return finish_move(&home, &moving, folder).map(Some);
    }
    let Ok(entries) = std::fs::read_dir(&home) else {
        return Ok(None);
    };
    Ok(entries
        .flatten()
        .map(|entry| entry.path())
        .find(|path| drive_marked_by(path, &stick.stick_id)))
}

#[cfg(target_os = "macos")]
pub fn find_moved(_stick: &OwnedStick, _folder: &str) -> Result<Option<PathBuf>> {
    Ok(None)
}

/// The disk image of a stick made before its image was recorded (macOS:
/// the one image of the app's first stick); `None` elsewhere.
#[cfg(target_os = "macos")]
pub fn image_of(_stick: &ourstick_core::OwnedStick) -> Option<PathBuf> {
    platform::image_path().ok()
}
#[cfg(not(target_os = "macos"))]
pub fn image_of(_stick: &ourstick_core::OwnedStick) -> Option<PathBuf> {
    None
}

/// What the app undoes for a stick being removed (ADR 0009 §5): its
/// bookmark or drive letter, or its disk image, detached then deleted.
pub struct AppDrive;

impl ourstick_core::stick_life::DriveOps for AppDrive {
    fn take_away(&self, stick: &ourstick_core::OwnedStick) -> Result<(), String> {
        eject(stick).map_err(|error| format!("{error:#}"))?;
        if let Some(image) = &stick.image_path {
            match std::fs::remove_file(image) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("cannot delete {}: {error}", image.display())),
            }
        }
        Ok(())
    }
}

/// Where to measure what is left for the stick (ADR 0008): the disk that
/// really holds its files. On macOS that is the disk image's folder, not the
/// mounted volume (which tells the image's own size).
#[cfg(target_os = "macos")]
pub use platform::measure_at;
#[cfg(not(target_os = "macos"))]
pub fn measure_at(stick: &OwnedStick) -> Result<PathBuf> {
    Ok(stick.root_path.clone())
}

/// Windows: the drive marker hidden (a dot does not hide a file there, and
/// a user deleting it would make the stick look ejected). Elsewhere a dot
/// hides it already.
#[cfg(windows)]
pub use platform::hide_marker;
#[cfg(not(windows))]
pub fn hide_marker(_root: &Path) {}

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
