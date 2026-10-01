//! Making an app stick: the one place, for the first launch and for
//! "New stick…" alike. Whatever fails, nothing half made is left behind.

use anyhow::{Context, Result};
use ourstick_core::places::check_new_stick_folder;
use ourstick_core::share::mark_drive;
use ourstick_core::stick_life::remove_stick;
use ourstick_core::stick_names::folder_name;
use ourstick_core::{device, OwnedStick, SecretStore, StickId, StickKind, Store};
use std::path::Path;

use crate::drive;

/// A stick just made: its id, and what could not be done after it was
/// made (its bookmark or letter), to tell; the stick stands.
pub struct Made {
    pub stick_id: StickId,
    pub unplugged: Option<String>,
}

/// Makes the app stick `name` (checked and in its one stored form): its
/// folder or disk image, keys and row, marker, then its bookmark or letter.
/// A failure before the stick is whole undoes what was made. Blocking; the
/// caller holds the setup lock.
pub fn make_stick(
    store: &dyn Store,
    secrets: &dyn SecretStore,
    data_dir: &Path,
    name: &str,
) -> Result<Made> {
    let home = drive::sticks_home()?;
    let folder = folder_name(name, |candidate| home.join(candidate).exists());
    // On macOS `home` is /Volumes and this only probes the volume name:
    // app sticks there are told apart by their images.
    check_new_stick_folder(store, data_dir, &home.join(&folder))?;
    let (root, image) = drive::create(&folder)?;
    let keys = match device::create_stick(
        store,
        secrets,
        &root,
        name,
        StickKind::App,
        image.as_deref(),
    ) {
        Ok(keys) => keys,
        Err(error) => {
            drive::discard(&root, image.as_deref());
            return Err(error.into());
        }
    };
    let stick_id = keys.stick.stick_id;
    if let Err(error) = mark_drive(&root, &stick_id) {
        // Its keys and row go as a removal does; then the empty folder.
        if let Err(undone) = remove_stick(store, secrets, data_dir, &stick_id, &drive::AppDrive) {
            tracing::warn!(error = %undone, "a stick that could not be made stays being removed");
        }
        drive::discard(&root, image.as_deref());
        return Err(error).context("cannot mark the stick's folder");
    }
    let unplugged = match plug_in(store, &keys.stick) {
        Ok(()) => None,
        Err(error) => Some(format!("{error:#}")),
    };
    Ok(Made {
        stick_id,
        unplugged,
    })
}

/// The new stick in the sidebar, or given its letter (kept, to ask for it
/// again after a restart).
fn plug_in(store: &dyn Store, stick: &OwnedStick) -> Result<()> {
    let root = drive::mount(stick)?;
    drive::hide_marker(&root);
    if let Some(letter) = drive::letter_of(&root) {
        store.set_drive_letter(&stick.stick_id, Some(letter))?;
    }
    Ok(())
}
