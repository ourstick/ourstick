//! The `Run` value, through `reg` (ADR 0023's way of writing the registry).

use std::path::Path;

use anyhow::Result;

use super::entries::{windows_file, windows_found, windows_removal, APPROVED, RUN};
use super::Found;
use crate::registry::platform::{import, query};

pub fn read(exe: &Path) -> Option<Found> {
    windows_found(query(RUN).as_deref(), query(APPROVED).as_deref(), exe)
}

fn data_dir() -> Result<std::path::PathBuf> {
    Ok(ourstick_core::device::data_dir()?)
}

pub fn write(exe: &Path, clear_off: bool) -> Result<()> {
    import(&data_dir()?, "login", &windows_file(exe, clear_off))
}

pub fn remove() -> Result<()> {
    import(&data_dir()?, "login", &windows_removal())
}
