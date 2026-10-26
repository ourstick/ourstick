//! A phone has no tray (ADR 0027): the window is the app.

use anyhow::Result;
use tauri::AppHandle;

use super::TrayThing;

pub fn create(_app: &AppHandle) -> Result<()> {
    Ok(())
}

pub fn set_active(_app: &AppHandle, _active: bool) {}

pub fn set_sticks(_app: &AppHandle, _sticks: Vec<TrayThing>, _start_at_login: bool) {}
