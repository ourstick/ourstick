//! The LaunchAgent, a file in `~/Library/LaunchAgents`; launchd reads it at
//! the next login.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::entries::{macos_found, macos_plist, MACOS_LABEL};
use super::Found;

fn path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("cannot find your home folder")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{MACOS_LABEL}.plist")))
}

pub fn read(exe: &Path) -> Option<Found> {
    macos_found(std::fs::read_to_string(path().ok()?).ok().as_deref(), exe)
}

pub fn write(exe: &Path, _clear_off: bool) -> Result<()> {
    let path = path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    std::fs::write(&path, macos_plist(exe))
        .with_context(|| format!("cannot write {}", path.display()))
}

pub fn remove() -> Result<()> {
    let path = path()?;
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("cannot delete {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Keeps macOS from starting the app again at login when it was running at
/// a restart ("Reopen windows when logging back in"): that start has no
/// `--autostarted`, so it would open the window. Only the LaunchAgent
/// starts it at login, as the user chose. Called on the main thread.
pub fn no_relaunch_at_login() {
    if let Some(main_thread) = objc2::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(main_thread).disableRelaunchOnLogin();
    }
}
