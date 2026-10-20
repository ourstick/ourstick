//! The app's sticks, and what a stick's life brings (ADR 0009 §5): the
//! list, a new link, removal. Each stops the stick's share first and keeps
//! it busy meanwhile, so nothing starts it halfway.

use ourstick_core::locks::{stick_lock, take_retrying};
use ourstick_core::stick_life::{give_new_link, remove_stick};
use ourstick_core::stick_names::{folder_name, new_stick_name};
use ourstick_core::{OwnedStick, StickId};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::{blocking, shown};
use crate::sharing::{make_busy, refresh_tray, start_sharing_later};
use crate::state::{stick_hex, stick_id as stick_id_of, AppState, Sharing};

/// One of the app's sticks, as the list shows it.
#[derive(Debug, Serialize)]
pub struct StickEntry {
    id: String,
    name: String,
    /// "off", "starting", "on", "failed" or "busy".
    sharing: &'static str,
    /// While busy, what is being done ("Renaming…").
    doing: Option<&'static str>,
    readers: usize,
}

/// The app's sticks, oldest first.
#[tauri::command]
pub async fn sticks(state: State<'_, AppState>) -> Result<Vec<StickEntry>, String> {
    let sticks = state.sticks_now().await.map_err(shown)?;
    let sharing = state.sticks.lock().await;
    Ok(sticks
        .into_iter()
        .map(|stick| {
            let now = sharing.get(&stick.stick_id);
            entry_of(stick, now)
        })
        .collect())
}

/// How the list shows `stick`, sharing as `now`.
fn entry_of(stick: OwnedStick, now: Option<&Sharing>) -> StickEntry {
    let (word, readers, doing) = match now {
        None | Some(Sharing::Off) => ("off", 0, None),
        Some(Sharing::Starting) => ("starting", 0, None),
        Some(Sharing::On(serving)) => ("on", serving.activity.borrow().readers, None),
        Some(Sharing::Failed(_)) => ("failed", 0, None),
        Some(Sharing::Busy(doing)) => ("busy", 0, Some(*doing)),
    };
    StickEntry {
        id: stick_hex(&stick.stick_id),
        name: stick.name,
        sharing: word,
        doing,
        readers,
    }
}

/// Keeps `stick_id` busy while it changes (see `make_busy`); the window,
/// the tray and its green dot show it at once.
async fn keep_busy(
    app: &AppHandle,
    state: &AppState,
    stick_id: &StickId,
    doing: &'static str,
) -> Result<(), String> {
    make_busy(state, stick_id, doing).await?;
    let _ = app.emit("sharing-changed", ());
    refresh_tray(app).await;
    Ok(())
}

/// After a change that kept `stick_id` busy: shared again in the
/// background if the user still wants it shared (read now, so a Stop
/// meanwhile counts), so the command answers at once.
async fn share_again_if_wanted(app: &AppHandle, state: &AppState, stick_id: StickId) {
    state.sticks.lock().await.remove(&stick_id);
    if state
        .stick_now(&stick_id)
        .await
        .is_ok_and(|stick| stick.sharing_wanted)
    {
        start_sharing_later(app, stick_id);
    }
}

/// Tells the window what could not be done for `stick_id` after a change
/// that stands.
pub fn tell_unplugged(app: &AppHandle, stick_id: StickId, unplugged: Option<String>) {
    if let Some(why) = unplugged {
        crate::problems::failed(
            app,
            Some(stick_id),
            format!("It is not shown as a drive ({why}); use Mount to try again."),
        );
    }
}

/// "Give it a new link": everyone with the old link loses it; the files
/// stay. Shared again with the new link if it was wanted shared. Returns the
/// stick's new id.
#[tauri::command]
pub async fn new_link(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let old = stick_id_of(&stick_id)?;
    let stick = state.stick_now(&old).await.map_err(shown)?;
    keep_busy(&app, &state, &old, "Changing the sharing link…").await?;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let made = blocking(move || {
        let secrets = crate::keys::secrets(store.as_ref())?;
        // The stick's own lock, as for every change of a stick; not the
        // device node key's, which another older stick may be using.
        let _lock = take_retrying(|| stick_lock(&data_dir, &stick))?;
        Ok(give_new_link(
            store.as_ref(),
            secrets.as_ref(),
            &data_dir,
            &old,
        )?)
    })
    .await;
    let new = match made {
        Ok(keys) => keys.stick.stick_id,
        Err(error) => {
            share_again_if_wanted(&app, &state, old).await;
            let _ = app.emit("sharing-changed", ());
            return Err(error);
        }
    };
    state.sticks.lock().await.remove(&old);
    // Windows: its File Explorer entry follows the new id (ADR 0023).
    if let Ok(stick) = state.stick_now(&new).await {
        if let Err(error) = blocking(move || crate::drive::link_renewed(&old, &stick)).await {
            tracing::warn!("{error}");
        }
    }
    crate::problems::tell(&app, Some(old), crate::serving::ShareProblem::Cleared);
    share_again_if_wanted(&app, &state, new).await;
    refresh_tray(&app).await;
    let _ = app.emit("sharing-changed", ());
    let _ = app.emit("sticks-changed", ());
    Ok(stick_hex(&new))
}

/// "Remove this stick": its link and keys go; no file of the user's is
/// deleted (ADR 0028 §7). Asked for in the window first. If it
/// cannot be removed, it is shared again as before.
#[tauri::command]
pub async fn remove(
    stick_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id = stick_id_of(&stick_id)?;
    let stick = state.stick_now(&id).await.map_err(shown)?;
    keep_busy(&app, &state, &id, "Removing…").await?;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let removed = blocking(move || {
        let secrets = crate::keys::secrets(store.as_ref())?;
        let lock = take_retrying(|| stick_lock(&data_dir, &stick))?;
        let removed = remove_stick(
            store.as_ref(),
            secrets.as_ref(),
            &data_dir,
            &id,
            &crate::drive::AppDrive,
        );
        drop(lock);
        Ok(removed?)
    })
    .await;
    match &removed {
        Ok(()) => {
            state.sticks.lock().await.remove(&id);
            crate::problems::tell(&app, Some(id), crate::serving::ShareProblem::Cleared);
        }
        // Not marked as being removed (refused early): still a stick.
        Err(_) => share_again_if_wanted(&app, &state, id).await,
    }
    refresh_tray(&app).await;
    let _ = app.emit("sharing-changed", ());
    let _ = app.emit("sticks-changed", ());
    let _ = app.emit("mount-changed", ());
    removed
}

/// Moves the first stick of an older version into `~/OurStick` if it is
/// `~/OurStick` itself, before a second stick is made there or it is
/// renamed (ADR 0009 §4). Its share stops meanwhile and starts again after,
/// if wanted; if it cannot move, it is plugged in again where it was.
async fn move_first_if_needed(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let home = blocking(crate::drive::sticks_home).await?;
    let first = state
        .sticks_now()
        .await
        .map_err(shown)?
        .into_iter()
        // Only a drive is ever moved: any other root is not OurStick's to move.
        .find(|stick| stick.is_drive() && stick.root_path == home);
    let Some(first) = first else {
        return Ok(());
    };
    let id = first.stick_id;
    keep_busy(app, state, &id, "Moving into the OurStick folder…").await?;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let moved = blocking(move || {
        let _lock = take_retrying(|| stick_lock(&data_dir, &first))?;
        // The new `~/OurStick` is empty: any name is free there.
        let folder = folder_name(&first.name, |_| false);
        let root = match crate::drive::move_first(&first, &folder) {
            Ok(root) => root,
            Err(error) => {
                let _ = crate::drive::mount(&first);
                return Err(error);
            }
        };
        store.move_owned_stick(&first.stick_id, &root)?;
        let moved = OwnedStick {
            root_path: root,
            ..first
        };
        crate::drive::mount(&moved)?;
        Ok(())
    })
    .await;
    share_again_if_wanted(app, state, id).await;
    let _ = app.emit("mount-changed", ());
    moved
}

/// "Rename…": the stick gets the name `name` (stored in one form, unique
/// among the app's sticks), its folder or volume and its bookmark or letter
/// follow, and readers see the new name with its next version. Its share
/// stops meanwhile and starts again after, if wanted. Windows refuses while
/// a file in it is open, and says so.
#[tauri::command]
pub async fn rename(
    stick_id: String,
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let id = stick_id_of(&stick_id)?;
    // The first stick of an older version is `~/OurStick` itself: renaming
    // it there would take it out of `~/OurStick`.
    move_first_if_needed(&app, &state).await?;
    let stick = state.stick_now(&id).await.map_err(shown)?;
    keep_busy(&app, &state, &id, "Renaming…").await?;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let renamed = blocking(move || {
        let _setup = ourstick_core::locks::setup_lock(&data_dir)?;
        let _lock = take_retrying(|| stick_lock(&data_dir, &stick))?;
        let others: Vec<OwnedStick> = crate::state::app_sticks(store.as_ref())?
            .into_iter()
            .filter(|other| other.stick_id != stick.stick_id)
            .collect();
        let name = new_stick_name(&name, &others)?;
        // A stick of ADR 0028 has no folder or drive to follow its name.
        if !stick.is_drive() {
            store.rename_owned_stick(&stick.stick_id, &name)?;
            return Ok(None);
        }
        // Next to where it is (on macOS a volume name, free in /Volumes).
        let around = match stick.root_path.parent() {
            Some(parent) if !cfg!(target_os = "macos") => parent.to_path_buf(),
            _ => crate::drive::sticks_home()?,
        };
        let current = stick.root_path.file_name().map(|name| name.to_owned());
        let folder = folder_name(&name, |candidate| {
            current.as_deref() != Some(std::ffi::OsStr::new(candidate))
                && around.join(candidate).exists()
        });
        let renamed = crate::drive::rename(&stick, &folder, &name)?;
        // Recorded at once: the folder has moved, whatever follows.
        if renamed.root != stick.root_path {
            store.move_owned_stick(&stick.stick_id, &renamed.root)?;
        }
        store.rename_owned_stick(&stick.stick_id, &name)?;
        if let Some(letter) = crate::drive::letter_of(&renamed.root) {
            store.set_drive_letter(&stick.stick_id, Some(letter))?;
        }
        Ok(renamed.unplugged)
    })
    .await;
    // A new version carries the new name to readers.
    share_again_if_wanted(&app, &state, id).await;
    let _ = app.emit("sticks-changed", ());
    let _ = app.emit("mount-changed", ());
    let _ = app.emit("sharing-changed", ());
    refresh_tray(&app).await;
    tell_unplugged(&app, id, renamed?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_says_what_a_busy_stick_is_doing() {
        let id = ourstick_core::StickSigningKey::from_bytes(&[5; 32]).stick_id();
        let stick = OwnedStick::new(id, "Photos", "/s", ourstick_core::CipherId::None, 0);
        let busy = entry_of(stick.clone(), Some(&Sharing::Busy("Renaming…")));
        assert_eq!((busy.sharing, busy.doing), ("busy", Some("Renaming…")));
        let off = entry_of(stick, None);
        assert_eq!((off.sharing, off.doing, off.readers), ("off", None, 0));
        assert_eq!(off.id, stick_hex(&id));
    }
}
