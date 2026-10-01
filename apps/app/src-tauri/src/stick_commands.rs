//! The stick's own commands: what is on it, copying onto it, linking to it
//! what does not fit (ADR 0008), removing. Each calls only `ourstick-core`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use ourstick_core::clock::{Clock, SystemClock};
use ourstick_core::drive::{copy_onto, remove_from, top_entries, DriveError};
use ourstick_core::layout::StickLayout;
use ourstick_core::linked::plan_link;
use ourstick_core::share::folder_size;
use ourstick_core::space::{available, SystemDisks};
use ourstick_core::{OwnedStick, Store};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::{blocking, shown};
use crate::drive;
use crate::state::{AppState, Sharing};

#[derive(Debug, Serialize)]
pub struct SharingView {
    /// "off", "starting", "on" or "failed".
    state: &'static str,
    /// Why it failed.
    message: Option<String>,
    /// The link can be shown (never sent anywhere by the app itself).
    has_link: bool,
    readers: usize,
    bytes_per_second: u64,
    /// What to do when readers come only through the relay.
    relay_only: Option<&'static str>,
}

/// One top-level entry of the stick.
#[derive(Debug, Serialize)]
pub struct EntryView {
    name: String,
    /// `None` for a linked entry not read yet.
    size: Option<u64>,
    is_dir: bool,
    /// Shared from where it is, not copied (ADR 0008).
    linked: bool,
    /// A linked entry that is not there (unplugged, moved): its last version
    /// is still offered.
    missing: bool,
    /// On the stick's folder, but not shared: a linked entry has its name.
    left_out: bool,
}

#[derive(Debug, Serialize)]
pub struct StickView {
    name: String,
    /// Bytes on it now (copied onto it; linked entries take no room).
    used: u64,
    /// What can still be copied onto it: what its disk has room for, every
    /// share's and download's claim counted (the space ledger).
    left: u64,
    entries: Vec<EntryView>,
    sharing: SharingView,
    /// Mounted (macOS) or in the sidebar (Linux); not after Eject.
    mounted: bool,
}

/// What can still be copied onto the stick whose folder is `root`. Blocking.
fn left_on(root: &Path, store: &dyn Store, data_dir: &Path) -> Result<u64> {
    let measure_at = drive::measure_at(root)?;
    Ok(available(&SystemDisks, store, data_dir, &measure_at, None)?)
}

/// The entries the window lists: the own folder's and the linked ones, by
/// name. A linked entry's size is what the last pass recorded (walking a
/// big linked folder at every look would be slow). Blocking.
fn entries_of(stick: &OwnedStick, store: &dyn Store) -> Result<Vec<EntryView>> {
    let layout = StickLayout::load(store, &stick.stick_id)?;
    let mut entries: Vec<EntryView> = Vec::new();
    // An ejected image has no folder: its own entries are not shown.
    if drive::folder_is_there(&stick.root_path) {
        for entry in top_entries(&stick.root_path)? {
            entries.push(EntryView {
                left_out: layout.is_shadowed(&entry.name),
                name: entry.name,
                size: Some(entry.size),
                is_dir: entry.is_dir,
                linked: false,
                missing: false,
            });
        }
    }
    let recorded = store.owned_files(&stick.stick_id)?;
    for root in layout.linked() {
        let name = root.name.clone().unwrap_or_default();
        let prefix = format!("{name}/");
        let mut files = recorded
            .iter()
            .filter(|file| file.path == name || file.path.starts_with(&prefix))
            .peekable();
        let size = files
            .peek()
            .is_some()
            .then(|| files.map(|file| file.stamp.size).sum());
        entries.push(EntryView {
            name,
            size,
            is_dir: root.is_dir,
            linked: true,
            missing: root.is_gone(),
            left_out: false,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.linked.cmp(&b.linked)));
    Ok(entries)
}

/// The stick as the main window shows it.
#[tauri::command]
pub async fn stick(state: State<'_, AppState>) -> Result<StickView, String> {
    let stick = state
        .drive_stick()
        .map_err(shown)?
        .ok_or("there is no stick yet")?;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let name = stick.name.clone();
    let (entries, used, left, mounted) = blocking(move || {
        let root = &stick.root_path;
        let mounted = drive::is_mounted(root);
        let left = left_on(root, store.as_ref(), &data_dir)?;
        let used = if drive::folder_is_there(root) {
            folder_size(root)?
        } else {
            0
        };
        Ok((entries_of(&stick, store.as_ref())?, used, left, mounted))
    })
    .await?;
    let sharing = match &*state.sharing.lock().await {
        Sharing::Off => sharing_view("off", None, false, None),
        Sharing::Starting => sharing_view("starting", None, false, None),
        Sharing::Failed(message) => sharing_view("failed", Some(message.clone()), false, None),
        Sharing::On(serving) => sharing_view(
            "on",
            None,
            serving.link.is_some(),
            Some(*serving.activity.borrow()),
        ),
    };
    Ok(StickView {
        name,
        used,
        left,
        entries,
        sharing,
        mounted,
    })
}

fn sharing_view(
    state: &'static str,
    message: Option<String>,
    has_link: bool,
    activity: Option<ourstick_core::transport::ReaderActivity>,
) -> SharingView {
    let activity = crate::serving::ActivityView::from(activity.unwrap_or_default());
    SharingView {
        state,
        message,
        has_link,
        readers: activity.readers,
        bytes_per_second: activity.bytes_per_second,
        relay_only: activity.relay_only,
    }
}

/// What became of a drop.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AddOutcome {
    Copied,
    /// Refused, as by a real stick: nothing was copied. The window offers
    /// to share it as it is.
    DoesNotFit {
        needed: u64,
        left: u64,
    },
}

/// Dropped files and folders: copied onto the stick, like onto a real one,
/// if they fit in what is left.
#[tauri::command]
pub async fn add_paths(
    paths: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AddOutcome, String> {
    let stick = state
        .drive_stick()
        .map_err(shown)?
        .ok_or("there is no stick yet")?;
    let sources: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let outcome = blocking(move || {
        if !drive::folder_is_there(&stick.root_path) {
            bail!("the stick is ejected; mount it first");
        }
        let left = left_on(&stick.root_path, store.as_ref(), &data_dir)?;
        let linked: Vec<String> = store
            .linked_entries(&stick.stick_id)?
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        match copy_onto(&stick.root_path, left, &linked, &sources) {
            Ok(_) => Ok(AddOutcome::Copied),
            Err(DriveError::DoesNotFit { needed, left }) => {
                Ok(AddOutcome::DoesNotFit { needed, left })
            }
            Err(error) => Err(error.into()),
        }
    })
    .await?;
    // The watcher publishes; the window can show the files at once.
    let _ = app.emit("stick-changed", ());
    Ok(outcome)
}

/// Tells the running share to look at the stick now (a linked entry came or
/// went: no file system event says so).
async fn nudge(state: &AppState) {
    if let Sharing::On(serving) = &*state.sharing.lock().await {
        serving.nudge.now();
    }
}

/// "Share it as it is": links dropped folders or files to the stick,
/// shared from where they are (ADR 0008). All or none: each is checked
/// first (no overlap with the stick, the data folder or another linked
/// entry; a name that is taken gets a number).
#[tauri::command]
pub async fn link_paths(
    paths: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let stick = state
        .drive_stick()
        .map_err(shown)?
        .ok_or("there is no stick yet")?;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    blocking(move || {
        let mut planned = store.linked_entries(&stick.stick_id)?;
        let known = planned.len();
        for path in paths {
            let entry = plan_link(
                &stick.root_path,
                &data_dir,
                &planned,
                Path::new(&path),
                SystemClock.now_ms(),
            )?;
            planned.push(entry);
        }
        for entry in &planned[known..] {
            store.add_linked_entry(&stick.stick_id, entry)?;
        }
        Ok(())
    })
    .await?;
    nudge(&state).await;
    let _ = app.emit("stick-changed", ());
    Ok(())
}

/// Removes a top-level entry: the stick's copy of a file or folder, or, for
/// a linked one, only the link (its files stay where they are).
#[tauri::command]
pub async fn remove_entry(
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let stick = state
        .drive_stick()
        .map_err(shown)?
        .ok_or("there is no stick yet")?;
    let store = state.store.clone();
    let unlinked = blocking(move || {
        let linked = store
            .linked_entries(&stick.stick_id)?
            .iter()
            .any(|entry| entry.name == name);
        if linked {
            store.remove_linked_entry(&stick.stick_id, &name)?;
        } else {
            remove_from(&stick.root_path, &name)?;
        }
        Ok(linked)
    })
    .await?;
    if unlinked {
        nudge(&state).await;
    }
    let _ = app.emit("stick-changed", ());
    Ok(())
}
