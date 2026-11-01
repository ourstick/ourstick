//! The stick's own commands: what is in it, adding to it from where things
//! are (ADR 0028 §3), taking out. Each calls only `ourstick-core`.

use std::path::Path;

use anyhow::Result;
use ourstick_core::clock::{Clock, SystemClock};
use ourstick_core::drive::top_entries;
use ourstick_core::layout::StickLayout;
use ourstick_core::linked::plan_link;
use ourstick_core::{OwnedStick, StickId, Store};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::{blocking, shown};
use crate::drive;
use crate::state::{stick_id as stick_id_of, AppState, Sharing};
use ourstick_core::sender_files::SenderFiles;

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
    /// Readers wait while their files are got ready.
    preparing: bool,
    /// What to do when readers come only through the relay.
    relay_only: Option<&'static str>,
    /// Sending slowly, so another program can use the disk.
    giving_way: bool,
    /// A reader on this network is kept on the relay by macOS (ADR 0020).
    local_network_declined: bool,
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
    /// Where a linked entry is: the name of the folder it is in.
    place: Option<String>,
    /// The one folder the stick was made from, its contents at the top:
    /// never taken out (ADR 0028 §2, amendment of 2026-10-20).
    at_top: bool,
}

#[derive(Debug, Serialize)]
pub struct StickView {
    name: String,
    entries: Vec<EntryView>,
    sharing: SharingView,
    /// A stick made before ADR 0028: a folder of its own, sent too.
    own_folder: bool,
    /// Its own folder is there (not an ejected image, an unplugged disk).
    folder_there: bool,
    /// Where its own files stay if it is removed: its folder, or its disk
    /// image on macOS. None for a stick of ADR 0028.
    folder: Option<String>,
    /// Its last problem (see `problems`), or none.
    problem: Option<crate::serving::ShareProblem>,
    /// When it was last looked at in this run, in seconds since 1970: a
    /// phone has no watcher and says so (ADR 0029 decision 4).
    looked_at: Option<u64>,
}

/// The entries the window lists: the own folder's and the linked ones, by
/// name. A linked entry's size is what the last pass recorded (walking a
/// big linked folder at every look would be slow). Blocking.
fn entries_of(
    sender: &dyn SenderFiles,
    stick: &OwnedStick,
    store: &dyn Store,
) -> Result<Vec<EntryView>> {
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
                place: None,
                at_top: false,
            });
        }
    }
    let recorded = store.owned_files(&stick.stick_id)?;
    for root in layout.linked() {
        let name = root.name.clone().unwrap_or_default();
        let prefix = format!("{name}/");
        // The folder at the top has every file no named entry has.
        let mut files = recorded
            .iter()
            .filter(|file| {
                if root.at_top {
                    !layout.is_named(file.path.split('/').next().unwrap_or_default())
                } else {
                    file.path == name || file.path.starts_with(&prefix)
                }
            })
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
            missing: root.is_gone(sender),
            left_out: false,
            at_top: root.at_top,
            // The folder it is in; none on a phone, whose chosen places
            // have no folder to show (ADR 0029).
            place: root
                .path
                .parent()
                .and_then(Path::file_name)
                .filter(|_| !cfg!(mobile))
                .map(|place| place.to_string_lossy().into_owned()),
        });
    }
    // The folder at the top first: the rest goes next to it.
    entries.sort_by(|a, b| {
        b.at_top
            .cmp(&a.at_top)
            .then(a.name.cmp(&b.name))
            .then(a.linked.cmp(&b.linked))
    });
    Ok(entries)
}

/// The stick as the main window shows it.
#[tauri::command]
pub async fn stick(
    stick_id: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<StickView, String> {
    let stick = state
        .stick_now(&stick_id_of(&stick_id)?)
        .await
        .map_err(shown)?;
    let store = state.store.clone();
    let sender = state.files.clone();
    let name = stick.name.clone();
    let id = stick.stick_id;
    let own_folder = stick.is_drive();
    let folder = own_folder.then(|| {
        stick
            .image_path
            .as_ref()
            .unwrap_or(&stick.root_path)
            .display()
            .to_string()
    });
    let (entries, folder_there) = blocking(move || {
        let there = drive::folder_is_there(&stick.root_path);
        Ok((entries_of(sender.as_ref(), &stick, store.as_ref())?, there))
    })
    .await?;
    let sharing = match state.sticks.lock().await.get(&id) {
        None | Some(Sharing::Off) => sharing_view("off", None, false, None),
        Some(Sharing::Starting) => sharing_view("starting", None, false, None),
        Some(Sharing::Busy(doing)) => sharing_view("busy", Some((*doing).to_owned()), false, None),
        Some(Sharing::Failed(message)) => {
            sharing_view("failed", Some(message.clone()), false, None)
        }
        Some(Sharing::On(serving)) => sharing_view(
            "on",
            None,
            serving.link.is_some(),
            Some(*serving.activity.borrow()),
        ),
    };
    Ok(StickView {
        name,
        entries,
        sharing,
        own_folder,
        folder_there,
        folder,
        problem: tauri::Manager::state::<crate::problems::Problems>(&app).of(Some(&id)),
        looked_at: tauri::Manager::state::<crate::serving::looked::LastLooked>(&app).of(&id),
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
        preparing: activity.preparing,
        relay_only: activity.relay_only,
        giving_way: activity.giving_way,
        local_network_declined: activity.local_network_declined,
    }
}

/// Tells the running share of `stick_id` to look at the stick now (a
/// linked entry came or went: no file system event says so).
pub(crate) async fn nudge(state: &AppState, stick_id: &StickId) {
    if let Some(Sharing::On(serving)) = state.sticks.lock().await.get(stick_id) {
        serving.nudge.now();
    }
}

/// "Look again": the stick is looked at now, as on a phone, which has no
/// watcher (ADR 0029 decision 4). The view hears of it ("stick-changed").
#[tauri::command]
pub async fn look_again(stick_id: String, state: State<'_, AppState>) -> Result<(), String> {
    nudge(&state, &stick_id_of(&stick_id)?).await;
    Ok(())
}

/// The app came back to the front on a phone: every stick it sends is
/// looked at now.
#[cfg_attr(not(mobile), allow(dead_code))]
pub async fn look_at_all(state: &AppState) {
    for sharing in state.sticks.lock().await.values() {
        if let Sharing::On(serving) = sharing {
            serving.nudge.now();
        }
    }
}

/// "Share it as it is": links dropped folders or files to the stick,
/// shared from where they are (ADR 0008). All or none: each is checked
/// first (no overlap with the stick, the data folder or another linked
/// entry; a name that is taken is refused, ADR 0028 §2).
#[tauri::command]
pub async fn link_paths(
    stick_id: String,
    paths: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id = stick_id_of(&stick_id)?;
    // Not into a stick being renamed, moved, given a new link or removed.
    if crate::busy::is_busy(&state, &id).await {
        return Err("The stick is being changed; share it as it is once that is done.".to_owned());
    }
    let stick = state.stick_now(&id).await.map_err(shown)?;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let files = state.files.clone();
    let handle = app.clone();
    blocking(move || {
        crate::send::check_adding(
            files.as_ref(),
            &handle,
            store.as_ref(),
            &data_dir,
            &stick.stick_id,
            &paths,
        )?;
        let mut planned = Vec::new();
        for path in paths {
            let entry = plan_link(
                files.as_ref(),
                store.as_ref(),
                &stick.stick_id,
                &data_dir,
                &planned,
                Path::new(&path),
                SystemClock.now_ms(),
            )?;
            planned.push(entry);
        }
        for entry in &planned {
            store.add_linked_entry(&stick.stick_id, entry)?;
        }
        Ok(())
    })
    .await?;
    nudge(&state, &stick.stick_id).await;
    let _ = app.emit("stick-changed", ());
    Ok(())
}

/// "Stop sending “Summer”?": takes a linked entry out of the stick; its
/// files stay where they are. Nothing of the user's is ever deleted (ADR
/// 0028 §3): an older stick's own files are changed in the file manager.
#[tauri::command]
pub async fn remove_entry(
    stick_id: String,
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let stick = state
        .stick_now(&stick_id_of(&stick_id)?)
        .await
        .map_err(shown)?;
    let store = state.store.clone();
    blocking(move || {
        let linked = store
            .linked_entries(&stick.stick_id)?
            .iter()
            .any(|entry| entry.name == name);
        if !linked {
            anyhow::bail!("Only what is sent from where it is can be taken out here.");
        }
        store.remove_linked_entry(&stick.stick_id, &name)?;
        Ok(())
    })
    .await?;
    nudge(&state, &stick.stick_id).await;
    let _ = app.emit("stick-changed", ());
    Ok(())
}

/// "2.8 GB · Pictures", clicked: a linked entry shown in the file manager.
#[tauri::command]
pub async fn show_entry(
    stick_id: String,
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let stick = state
        .stick_now(&stick_id_of(&stick_id)?)
        .await
        .map_err(shown)?;
    let store = state.store.clone();
    let path = blocking(move || {
        let entry = store
            .linked_entries(&stick.stick_id)?
            .into_iter()
            .find(|entry| entry.name == name);
        // A phone has no file manager to show it in (ADR 0029).
        if cfg!(mobile) {
            anyhow::bail!("A phone cannot show it in a file manager.");
        }
        match entry {
            Some(entry) if entry.path.exists() => Ok(entry.path),
            _ => anyhow::bail!(
                "It is not there any more (moved or deleted, or its disk is unplugged)."
            ),
        }
    })
    .await?;
    app.opener().reveal_item_in_dir(path).map_err(|error| {
        tracing::warn!(%error, "the file manager did not show an entry");
        "It could not be shown in the file manager.".to_owned()
    })
}
