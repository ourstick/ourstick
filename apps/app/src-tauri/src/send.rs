//! Send (ADR 0028 §2): what is chosen is checked by the core's one place
//! (`places::may_send`), counted, then made into a new stick of linked
//! entries, all or none, shared at once.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use ourstick_core::clock::{Clock, SystemClock};
use ourstick_core::linked::{check_next_to_top, name_clash, one_folder_at_top, plan_link};
use ourstick_core::places::{
    app_places, may_send, measure as measure_path, Ask, MaySend, Measured, SendRules,
};
use ourstick_core::sender_files::SenderFiles;
use ourstick_core::stick_life::remove_stick;
use ourstick_core::stick_names::{new_stick_name, NameError};
use ourstick_core::LinkedEntry;
use ourstick_core::Store;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::blocking;
use crate::sharing::{refresh_tray, start_sharing_later};
use crate::state::{stick_hex, stick_id as stick_id_of, AppState};

/// What the window may do with one chosen path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Verdict {
    path: String,
    /// What it is called in the list (its last name).
    name: String,
    is_dir: bool,
    /// "fine", "ask" or "refuse".
    kind: &'static str,
    /// Refused: why, for the user.
    why: Option<String>,
    /// Asked: "main" (a main folder of the home) or "disk", and the question.
    ask: Option<&'static str>,
    question: Option<String>,
}

/// The verdicts on `paths`, in order: each by `rules`, with what is chosen
/// already (`rules.chosen`, `taken`: names and paths, and the folder at the
/// top of the stick added to, if it has one) and the paths before it that
/// are not refused.
fn verdicts(
    files: &dyn SenderFiles,
    paths: &[String],
    mut rules: SendRules,
    mut taken: Vec<(String, PathBuf)>,
    top: Option<&LinkedEntry>,
) -> Vec<Verdict> {
    let mut verdicts = Vec::new();
    for text in paths {
        let path = Path::new(text);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| text.clone());
        let verdict = |kind, why, ask, question| Verdict {
            path: text.clone(),
            name: name.clone(),
            is_dir: files
                .facts_followed(path)
                .is_ok_and(|facts| facts.is_folder()),
            kind,
            why,
            ask,
            question,
        };
        let decided = match may_send(files, path, &rules) {
            MaySend::Refuse(why) => verdict("refuse", Some(why), None, None),
            judged => {
                let clash = top
                    .and_then(|top| check_next_to_top(files, top, &name).err())
                    .or_else(|| {
                        name_clash(
                            path,
                            taken
                                .iter()
                                .map(|(name, path)| (name.as_str(), path.as_path())),
                        )
                    });
                match (clash, judged) {
                    (Some(clash), _) => verdict("refuse", Some(clash.to_string()), None, None),
                    (None, MaySend::Ask(ask)) => {
                        let kind = match ask {
                            Ask::MainFolder { .. } => "main",
                            Ask::WholeDisk { .. } => "disk",
                        };
                        verdict("ask", None, Some(kind), Some(ask.question()))
                    }
                    (None, _) => verdict("fine", None, None, None),
                }
            }
        };
        if decided.kind != "refuse" {
            rules.chosen.push(path.to_path_buf());
            taken.push((name.clone(), path.to_path_buf()));
        }
        verdicts.push(decided);
    }
    verdicts
}

/// This device's rules: its home and main folders (the system's own list,
/// so localized names too), and OurStick's own places.
fn rules_here(app: &AppHandle, store: &dyn Store, data_dir: &Path) -> Result<SendRules> {
    let paths = app.path();
    let home = paths.home_dir()?;
    let main_folders = [
        paths.document_dir(),
        #[cfg(desktop)]
        paths.desktop_dir(),
        paths.picture_dir(),
        paths.download_dir(),
        paths.audio_dir(),
        paths.video_dir(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let own = app_places(store, data_dir, None)?;
    Ok(SendRules::of_this_device(&home, main_folders, own))
}

/// What the stick `stick_id` sends already, if adding to it: its entries'
/// names and paths (two never share a name, the one at the top included),
/// and the folder at its top, whose contents' names are taken too.
struct Sent {
    taken: Vec<(String, PathBuf)>,
    paths: Vec<PathBuf>,
    top: Option<LinkedEntry>,
}

fn sent_by(store: &dyn Store, stick_id: Option<&ourstick_core::StickId>) -> Result<Sent> {
    let entries = match stick_id {
        Some(id) => store.linked_entries(id)?,
        None => Vec::new(),
    };
    Ok(Sent {
        taken: entries
            .iter()
            .map(|entry| (entry.name.clone(), entry.path.clone()))
            .collect(),
        paths: entries.iter().map(|entry| entry.path.clone()).collect(),
        top: entries.into_iter().find(|entry| entry.at_top),
    })
}

/// Checks `paths` for Send (or `+ Add more` on the stick `stick_id`), with
/// `chosen` in the list already.
#[tauri::command]
pub async fn check_send(
    paths: Vec<String>,
    chosen: Vec<String>,
    stick_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<Verdict>, String> {
    let id = stick_id.as_deref().map(stick_id_of).transpose()?;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let files = state.files.clone();
    blocking(move || {
        let mut rules = rules_here(&app, store.as_ref(), &data_dir)?;
        let Sent {
            mut taken,
            paths: sent,
            top,
        } = sent_by(store.as_ref(), id.as_ref())?;
        rules.chosen = sent;
        for path in chosen.iter().map(PathBuf::from) {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            if let Some(name) = name {
                taken.push((name, path.clone()));
            }
            rules.chosen.push(path);
        }
        Ok(verdicts(files.as_ref(), &paths, rules, taken, top.as_ref()))
    })
    .await
}

/// Counts what is at `path`, telling the window as it goes.
#[tauri::command]
pub async fn measure(
    path: String,
    so_far: Channel<Measured>,
    state: State<'_, AppState>,
) -> Result<Measured, String> {
    let files = state.files.clone();
    blocking(move || {
        Ok(measure_path(
            files.as_ref(),
            Path::new(&path),
            500,
            &mut |measured| {
                let _ = so_far.send(measured);
            },
        ))
    })
    .await
}

/// Refuses `paths` if one is refused (the window asked about the others).
fn check_all(
    files: &dyn SenderFiles,
    paths: &[String],
    rules: SendRules,
    taken: Vec<(String, PathBuf)>,
    top: Option<&LinkedEntry>,
) -> Result<()> {
    if let Some(refused) = verdicts(files, paths, rules, taken, top)
        .into_iter()
        .find(|verdict| verdict.kind == "refuse")
    {
        bail!("{}", refused.why.unwrap_or_default());
    }
    Ok(())
}

/// Refuses adding `paths` to the stick `stick_id` (`+ Add more`) if one
/// is refused, with what it sends already. Blocking.
pub(crate) fn check_adding(
    files: &dyn SenderFiles,
    app: &AppHandle,
    store: &dyn Store,
    data_dir: &Path,
    stick_id: &ourstick_core::StickId,
    paths: &[String],
) -> Result<()> {
    let Sent {
        taken,
        paths: sent,
        top,
    } = sent_by(store, Some(stick_id))?;
    let mut rules = rules_here(app, store, data_dir)?;
    rules.chosen = sent;
    check_all(files, paths, rules, taken, top.as_ref())
}

/// Why a name cannot be used, in the window's words (it never says stick).
fn name_words(error: &NameError) -> &'static str {
    match error {
        NameError::Empty => "Give it a name first.",
        NameError::TooLong => "The name can have at most 60 characters.",
        NameError::BadCharacter => "The name cannot hold invisible characters.",
        NameError::Taken(_) => "You already send something with this name. Choose another.",
    }
}

/// "Get the link": a new stick called `name`, with no folder of its own,
/// sending `paths` from where they are (one folder alone at its top); all
/// or none. Shared at once.
/// Returns its id.
#[tauri::command]
pub async fn send(
    name: String,
    paths: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    if paths.is_empty() {
        return Err("Choose something to send first.".to_owned());
    }
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let files = state.files.clone();
    let handle = app.clone();
    let made = blocking(move || {
        check_all(files.as_ref(), &paths, rules_here(&handle, store.as_ref(), &data_dir)?, Vec::new(), None)?;
        crate::keys::ensure_device(store.as_ref(), &data_dir)?;
        let secrets = crate::keys::secrets(store.as_ref())?;
        // One program makes a stick at a time: names stay unique.
        let _setup = ourstick_core::locks::setup_lock(&data_dir)?;
        let others = crate::state::app_sticks(store.as_ref())?;
        let name = new_stick_name(&name, &others).map_err(|error| anyhow::anyhow!(name_words(&error)))?;
        let keys = ourstick_core::device::create_sending_stick(store.as_ref(), secrets.as_ref(), &data_dir, &name)?;
        let id = keys.stick.stick_id;
        let linked = (|| -> Result<()> {
            let mut planned: Vec<LinkedEntry> = Vec::new();
            for path in &paths {
                let entry = plan_link(files.as_ref(), store.as_ref(), &id, &data_dir, &planned, Path::new(path), SystemClock.now_ms())?;
                planned.push(entry);
            }
            one_folder_at_top(&mut planned);
            for entry in &planned {
                store.add_linked_entry(&id, entry)?;
            }
            Ok(())
        })();
        if let Err(error) = linked {
            // Nothing half made: its keys and row go as a removal does.
            if let Err(undone) = remove_stick(store.as_ref(), secrets.as_ref(), &data_dir, &id, &crate::drive::AppDrive) {
                tracing::warn!(error = %undone, "a stick that could not be made stays being removed");
            }
            return Err(error);
        }
        Ok(id)
    })
    .await?;
    let _ = app.emit("sticks-changed", ());
    refresh_tray(&app).await;
    // The device may be new: the Local Network step is kept with it.
    crate::local_network::remember(&state);
    start_sharing_later(&app, made);
    Ok(stick_hex(&made))
}

/// What to choose with the system's picker: files, or folders
/// (Linux's and Windows' pickers do not take both at once).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pick {
    Files,
    Folders,
}

/// `Choose folders…` / `Choose files…`: the system's picker, several at
/// once; what is chosen is then checked as a drop is. Empty if the user
/// cancelled.
#[cfg(desktop)]
#[tauri::command]
pub async fn choose_paths(pick: Pick, app: AppHandle) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file().set_title("Choose what to send");
        match pick {
            Pick::Files => dialog.blocking_pick_files(),
            Pick::Folders => dialog.blocking_pick_folders(),
        }
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|picked| picked.into_path().ok())
        .map(|path| path.display().to_string())
        .collect())
}

/// Sending from a phone is not there yet (ADR 0027).
#[cfg(mobile)]
#[tauri::command]
pub async fn choose_paths(_pick: Pick, _app: AppHandle) -> Result<Vec<String>, String> {
    Err("A phone cannot send yet.".to_owned())
}

#[cfg(test)]
#[path = "send_tests.rs"]
mod tests;
