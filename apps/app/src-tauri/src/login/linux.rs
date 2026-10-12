//! The XDG autostart entry, `$XDG_CONFIG_HOME/autostart/ourstick.desktop`
//! (else `~/.config/autostart/`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::entries::{linux_entry, linux_found, LINUX_FILE};
use super::Found;

fn path() -> Result<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let config = match set("XDG_CONFIG_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(set("HOME").context("cannot find your home folder")?).join(".config"),
    };
    Ok(config.join("autostart").join(LINUX_FILE))
}

pub fn read(exe: &Path) -> Option<Found> {
    linux_found(std::fs::read_to_string(path().ok()?).ok().as_deref(), exe)
}

/// Written anew: also what clears the desktop's own "off".
pub fn write(exe: &Path, _clear_off: bool) -> Result<()> {
    let path = path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    std::fs::write(&path, linux_entry(exe))
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
