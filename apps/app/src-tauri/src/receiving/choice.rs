//! Choosing what to get of a received stick (ADR 0012 and its Amendment
//! 1): its top-level entries, or a top-level folder's, with their ticks, and
//! saving a choice.

use ourstick_core::device::open_system_keyring;
use ourstick_core::download::{entries_in, LeftOut, Tick};
use ourstick_core::store::Store;
use ourstick_core::untrusted::escape;
use ourstick_core::Manifest;
use ourstick_core::StickId;
use serde::Serialize;
use tauri::{AppHandle, State};

use super::{read_ticket, start, Receivers};
use crate::state::{stick_id as stick_id_of, AppState};

/// One entry of a received stick, as the choice lists it: at its top level,
/// or inside a top-level folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceEntry {
    /// Exactly as the stick names it, the whole path (`Folder/Sub`): what a
    /// choice is saved with.
    name: String,
    /// Its last part, escaped for the window (the sender's text).
    shown: String,
    is_dir: bool,
    files: u64,
    bytes: u64,
    /// "on", "off", or "part" (a folder with some of its inside off), as
    /// `LeftOut` says.
    tick: &'static str,
    /// Files of it are already here (they stay if it is un-ticked).
    here: bool,
    /// On the stick now; an un-ticked name the stick no longer has is still
    /// listed, so a save never drops it.
    on_stick: bool,
}

/// The word for a tick, as the window takes it.
pub fn tick_word(tick: Tick) -> &'static str {
    match tick {
        Tick::On => "on",
        Tick::Off => "off",
        Tick::Part => "part",
    }
}

/// The last part of a path, escaped for the window.
fn shown(path: &str) -> String {
    escape(path.rsplit('/').next().unwrap_or(path))
}

/// What a received stick has at its top level (`folder` none) or inside a
/// top-level folder, with what is ticked: from the latest round's manifest
/// (none yet if no round has read it since the app started), together with
/// every path left out at that level that the stick no longer has (all of
/// them when the manifest is not known), so a save never drops one. Largest
/// first.
pub fn choice_entries(
    store: &dyn Store,
    stick_id: &StickId,
    manifest: Option<&Manifest>,
    folder: Option<&str>,
) -> anyhow::Result<Vec<ChoiceEntry>> {
    // A row saved before a check refused it is not shown, so a save never
    // sends it back.
    let paths = LeftOut::valid_only(&store.left_out(stick_id)?);
    let left_out = LeftOut::new(&paths);
    let mut entries: Vec<ChoiceEntry> = Vec::new();
    if let Some(manifest) = manifest {
        for entry in entries_in(manifest, folder, None) {
            entries.push(ChoiceEntry {
                shown: shown(&entry.name),
                is_dir: entry.is_dir,
                files: entry.files,
                bytes: entry.bytes,
                tick: tick_word(left_out.tick_of(&entry.name)),
                here: store.has_files_under(stick_id, &entry.name)?,
                on_stick: true,
                name: entry.name,
            });
        }
    }
    let at_this_level = |path: &str| match (manifest, folder) {
        (None, _) => folder.is_none(),
        (Some(_), None) => !path.contains('/'),
        (Some(_), Some(folder)) => path.rsplit_once('/').map(|(parent, _)| parent) == Some(folder),
    };
    for path in paths.iter().filter(|path| at_this_level(path)) {
        if !entries.iter().any(|entry| &entry.name == path) {
            entries.push(ChoiceEntry {
                name: path.clone(),
                shown: if manifest.is_none() {
                    escape(path)
                } else {
                    shown(path)
                },
                is_dir: false,
                files: 0,
                bytes: 0,
                tick: "off",
                here: store.has_files_under(stick_id, path)?,
                on_stick: false,
            });
        }
    }
    entries.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

/// A path left out, as the list says it: "Not getting: Films", or "Not
/// updating" for one whose files are already here; one inside a folder is
/// told with it ("MUSIC and Pictures (inside Disk)").
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct LeftOutView {
    shown: String,
    here: bool,
    /// The folder it is inside, escaped; none at the top level.
    inside: Option<String>,
}

/// What a received stick leaves out, as its line in the list says it.
pub fn left_out_views(store: &dyn Store, stick_id: &StickId) -> anyhow::Result<Vec<LeftOutView>> {
    let mut views = Vec::new();
    for name in LeftOut::valid_only(&store.left_out(stick_id)?) {
        views.push(LeftOutView {
            shown: shown(&name),
            here: store.has_files_under(stick_id, &name)?,
            inside: name.rsplit_once('/').map(|(folder, _)| escape(folder)),
        });
    }
    Ok(views)
}

/// Saves a choice: the top-level names not to get (each a plain name; the
/// whole set replaces the last). The folder to receive the stick into again
/// if it is not paused (paused, the choice counts when it goes on).
pub fn save_choice(
    store: &dyn Store,
    stick_id: &StickId,
    left_out: &[String],
) -> anyhow::Result<Option<std::path::PathBuf>> {
    if LeftOut::normalised(left_out).is_err() {
        anyhow::bail!("That choice cannot be saved. Choose again.");
    }
    store.set_left_out(stick_id, left_out)?;
    let joined = store
        .joined_stick(stick_id)?
        .ok_or_else(|| anyhow::anyhow!("This stick is not received here any more."))?;
    Ok((!joined.paused).then_some(joined.output_dir))
}

/// What the choice window lists for a received stick.
#[derive(Debug, Serialize)]
pub struct ChoiceView {
    entries: Vec<ChoiceEntry>,
    /// A round has read the stick's list since the app started: every entry
    /// is here, not only those already left out.
    known: bool,
    /// Every path left out, shown here or not (inside a folder not opened,
    /// or one the stick no longer has): the window starts from it, so a
    /// save never drops one.
    left_out: Vec<String>,
    /// Those of them whose files are already here, for "stays in your
    /// folder" while their folder is not opened.
    staying: Vec<Staying>,
}

/// A path left out whose files are already here.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Staying {
    name: String,
    /// Its last part, escaped.
    shown: String,
}

/// The paths of `left_out` whose files are already here.
pub fn staying(
    store: &dyn Store,
    stick_id: &StickId,
    left_out: &[String],
) -> anyhow::Result<Vec<Staying>> {
    let mut here = Vec::new();
    for name in left_out {
        if store.has_files_under(stick_id, name)? {
            here.push(Staying {
                shown: shown(name),
                name: name.clone(),
            });
        }
    }
    Ok(here)
}

/// "Choose what to get…": the received stick's top-level entries, or a
/// top-level `folder`'s when the window opens it.
#[tauri::command]
pub async fn received_contents(
    stick_id: String,
    folder: Option<String>,
    state: State<'_, AppState>,
    receivers: State<'_, Receivers>,
) -> Result<ChoiceView, String> {
    let stick_id = stick_id_of(&stick_id)?;
    let manifest = receivers.contents(&stick_id);
    let known = manifest.is_some();
    let store = state.store.clone();
    let (entries, left_out, staying) = super::work(move || {
        let entries = choice_entries(
            store.as_ref(),
            &stick_id,
            manifest.as_deref(),
            folder.as_deref(),
        )?;
        let left_out = LeftOut::valid_only(&store.left_out(&stick_id)?);
        let here = staying(store.as_ref(), &stick_id, &left_out)?;
        Ok((entries, left_out, here))
    })
    .await?;
    Ok(ChoiceView {
        entries,
        known,
        left_out,
        staying,
    })
}

/// Saves what the user chose not to get (the whole set), then receives the
/// stick again with it, unless it is paused (the choice counts when it goes
/// on).
#[tauri::command]
pub async fn choose_what_to_get(
    stick_id: String,
    left_out: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
    receivers: State<'_, Receivers>,
) -> Result<(), String> {
    let stick_id = stick_id_of(&stick_id)?;
    if !receivers.is_held(&stick_id).await {
        return Err("This stick is not received here any more.".to_owned());
    }
    let store = state.store.clone();
    let again = super::work(move || {
        let Some(folder) = save_choice(store.as_ref(), &stick_id, &left_out)? else {
            return Ok(None);
        };
        let secrets = open_system_keyring()?;
        Ok(Some((read_ticket(secrets.as_ref(), &stick_id)?, folder)))
    })
    .await?;
    if let Some((ticket, folder)) = again {
        start(&app, stick_id, ticket, folder, false).await;
    }
    super::changed(&app);
    Ok(())
}

#[cfg(test)]
#[path = "choice_tests.rs"]
mod tests;
