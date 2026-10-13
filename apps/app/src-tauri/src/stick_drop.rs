//! A drop onto a stick: copied, like onto a real USB stick, if it fits; a
//! name already there asked about first; one copy at a time per stick, its
//! bytes told as it goes (`docs/gui-spec.md`, the drop zone).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::bail;
use ourstick_core::drive::{
    check_sources, copy_onto_stoppable, names_taken, remove_leftovers, CopyStep, DriveError,
    OnConflict,
};
use ourstick_core::StickId;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::{blocking, shown};
use crate::drive;
use crate::state::{stick_id as stick_id_of, AppState};
use crate::stick_commands::left_on;

/// What became of a drop.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AddOutcome {
    Copied,
    /// Names already on the stick (copies, or linked entries): nothing was
    /// copied; the window asks Replace / Keep both / Cancel, then drops
    /// again with the answer.
    AlreadyThere {
        names: Vec<String>,
    },
    /// Refused, as by a real stick: nothing was copied. The window offers
    /// to share it as it is.
    DoesNotFit {
        needed: u64,
        left: u64,
    },
    /// Stopped by the user (Cancel, or Share it as it is): nothing of it
    /// is on the stick.
    Stopped,
}

/// Dropped files and folders: copied onto the stick, like onto a real one,
/// if they fit in what is left. A name already there is asked about first
/// (`on_conflict` none: "already_there"; "replace" or "keep_both" after the
/// answer). One copy at a time per stick; its bytes are told as
/// `copy-progress`.
#[tauri::command]
pub async fn add_paths(
    stick_id: String,
    paths: Vec<String>,
    on_conflict: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AddOutcome, String> {
    let id = stick_id_of(&stick_id)?;
    let on_conflict = answer(on_conflict.as_deref())?;
    // Marked first, the stick read after: a change that ended in between
    // (a removal) is seen, never copied into the folder it left.
    let copying = Copying::may_start(&state, id).await?;
    let stop = copying.stop.clone();
    let stick = state.stick_now(&id).await.map_err(shown)?;
    let sources: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let progress_app = app.clone();
    let outcome = blocking(move || {
        if !drive::folder_is_there(&stick.root_path) {
            bail!("the stick is ejected; mount it first");
        }
        // What a copy cut short left takes no room from this one.
        remove_leftovers(&stick.root_path);
        let left = left_on(&stick, store.as_ref(), &data_dir)?;
        let linked: Vec<String> = store
            .linked_entries(&stick.stick_id)?
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        // What is on the stick already is refused before anything is asked:
        // replacing it would delete what is being copied.
        check_sources(&stick.root_path, &sources)?;
        let on_conflict = match on_conflict {
            Some(answered) => answered,
            None => {
                let names = names_taken(&stick.root_path, &sources);
                if !names.is_empty() {
                    return Ok(AddOutcome::AlreadyThere { names });
                }
                OnConflict::KeepBoth
            }
        };
        let mut told = std::time::Instant::now();
        let mut progress = |step: CopyStep| {
            let finished = matches!(step, CopyStep::Copying(copied) if copied.done == copied.total);
            // A few times a second is enough for a bar.
            if told.elapsed() >= std::time::Duration::from_millis(250) || finished {
                told = std::time::Instant::now();
                let _ = progress_app.emit(
                    "copy-progress",
                    crate::serving::StickEvent::new(&id, CopyView::from(step)),
                );
            }
        };
        match copy_onto_stoppable(
            &stick.root_path,
            left,
            &linked,
            &sources,
            on_conflict,
            &stop,
            &mut progress,
        ) {
            Ok(_) => Ok(AddOutcome::Copied),
            Err(DriveError::DoesNotFit { needed, left }) => {
                Ok(AddOutcome::DoesNotFit { needed, left })
            }
            Err(DriveError::Stopped) => Ok(AddOutcome::Stopped),
            Err(error) => Err(error.into()),
        }
    })
    .await;
    // The watcher publishes; the window can show the files at once. Also
    // after a copy that failed: what came of it is on the stick.
    let _ = app.emit("stick-changed", ());
    outcome
}

/// What to choose with the system's picker for a stick: files, or folders
/// (Linux's and Windows' pickers do not take both at once).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pick {
    Files,
    Folders,
}

/// "Click to choose": the system's picker, several at once; what is chosen
/// is then added as a drop is. Empty if the user cancelled.
#[tauri::command]
pub async fn choose_for_stick(pick: Pick, app: AppHandle) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file().set_title("Add to the stick");
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

/// The window's answer to "already on the stick", if any.
fn answer(answered: Option<&str>) -> Result<Option<OnConflict>, String> {
    match answered {
        None => Ok(None),
        Some("replace") => Ok(Some(OnConflict::Replace)),
        Some("keep_both") => Ok(Some(OnConflict::KeepBoth)),
        Some(_) => Err("That answer is not one of the choices.".to_owned()),
    }
}

/// How far a drop is, for the window: while what was dropped is looked at
/// (`measuring`: the files and bytes counted so far, in `files` and
/// `total`), then while it is copied (`done` of `total`).
#[derive(Debug, Clone, Copy, Serialize)]
struct CopyView {
    measuring: bool,
    files: u64,
    done: u64,
    total: u64,
}

impl From<CopyStep> for CopyView {
    fn from(step: CopyStep) -> Self {
        match step {
            CopyStep::Measuring { files, bytes } => Self {
                measuring: true,
                files,
                done: 0,
                total: bytes,
            },
            CopyStep::Copying(copied) => Self {
                measuring: false,
                files: 0,
                done: copied.done,
                total: copied.total,
            },
        }
    }
}

/// Cancel, or Share it as it is, while a drop is looked at or copied: it
/// stops at its next file or piece, and nothing of it stays on the stick.
#[tauri::command]
pub fn stop_copy(stick_id: String, state: State<'_, AppState>) -> Result<(), String> {
    stop_by(&state, &stick_id)
}

/// Stops the copy onto the stick `stick_id` (hex), if one runs.
fn stop_by(state: &AppState, stick_id: &str) -> Result<(), String> {
    let id = stick_id_of(stick_id)?;
    if let Some(stop) = state
        .copying
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&id)
    {
        stop.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// What a copy cut short by a Quit left on `stick` is removed, unless a drop
/// copies onto it now (its own hidden folder would go): that drop clears it.
/// Blocking.
pub fn clear_cut_copy(state: &AppState, stick: &ourstick_core::OwnedStick) {
    if let Ok(_copying) = Copying::start(state, stick.stick_id) {
        if drive::folder_is_there(&stick.root_path) {
            remove_leftovers(&stick.root_path);
        }
    }
}

/// Whether a drop is copying onto `stick_id` now.
pub fn is_copying(state: &AppState, stick_id: &StickId) -> bool {
    state
        .copying
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains_key(stick_id)
}

/// A stick being copied onto, until dropped: one copy at a time per stick.
struct Copying<'a> {
    state: &'a AppState,
    stick_id: StickId,
    /// Set to stop this copy (`stop_copy`).
    stop: Arc<AtomicBool>,
}

impl<'a> Copying<'a> {
    /// A copy onto `stick_id` may start: none goes on, and the stick is not
    /// being changed (renamed, given a new link, moved, removed), which
    /// would leave the copy in a folder that is no longer the stick's. The
    /// copy is marked first, then the stick is checked (`make_busy` checks
    /// in the other order), so the two never both go ahead.
    async fn may_start(state: &'a AppState, stick_id: StickId) -> Result<Self, String> {
        let copying = Self::start(state, stick_id)?;
        if crate::busy::is_busy(state, &stick_id).await {
            return Err(
                "The stick is being changed (renamed, moved, given a new link or removed); \
                 drop again once that is done."
                    .to_owned(),
            );
        }
        Ok(copying)
    }

    fn start(state: &'a AppState, stick_id: StickId) -> Result<Self, String> {
        let mut copying = state
            .copying
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if copying.contains_key(&stick_id) {
            return Err("Wait until the copy is done.".to_owned());
        }
        let stop = Arc::new(AtomicBool::new(false));
        copying.insert(stick_id, stop.clone());
        Ok(Self {
            state,
            stick_id,
            stop,
        })
    }
}

impl Drop for Copying<'_> {
    fn drop(&mut self) {
        self.state
            .copying
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.stick_id);
    }
}

#[cfg(test)]
#[path = "stick_drop_tests.rs"]
mod tests;
