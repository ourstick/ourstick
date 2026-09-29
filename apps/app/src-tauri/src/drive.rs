//! The stick's folder, shown as a drive (docs/gui-spec.md). Linux: the folder
//! `~/OurStick`. macOS: for now the same; a disk image replaces it.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use ourstick_core::manifest::ignore::is_ignored;
use ourstick_core::manifest::DRIVE_MARKER;

/// The folder of a new stick. An existing `~/OurStick` is used only if it is
/// empty (system files aside) or already a stick's drive; anything else is
/// the user's and is left alone.
pub fn create() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("cannot find your home folder")?;
    let root = PathBuf::from(home).join("OurStick");
    if root.exists() {
        let mut entries =
            std::fs::read_dir(&root).with_context(|| format!("cannot open {}", root.display()))?;
        let is_drive = root.join(DRIVE_MARKER).exists();
        let has_own_files = entries.any(|entry| {
            entry.is_ok_and(|entry| !entry.file_name().to_str().is_some_and(is_ignored))
        });
        if has_own_files && !is_drive {
            bail!(
                "{} already exists and holds files. OurStick will not take it over; \
                 move it away and try again.",
                root.display()
            );
        }
    } else {
        std::fs::create_dir(&root).with_context(|| format!("cannot create {}", root.display()))?;
    }
    Ok(root)
}
