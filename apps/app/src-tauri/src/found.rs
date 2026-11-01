//! "Find it…" (ADR 0029): an entry of a stick that is not found where it
//! was is chosen again where it is now. It keeps its name and the link.

use std::path::Path;

use ourstick_core::linked::plan_found;
use tauri::{AppHandle, Emitter, State};

use crate::commands::{blocking, shown};
use crate::state::{stick_id as stick_id_of, AppState};

/// Asks where the entry `name` of the stick `stick_id` is now, with the
/// system's picker, and sends it from there. False if the user cancelled.
#[tauri::command]
pub async fn find_entry(
    stick_id: String,
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let stick = state
        .stick_now(&stick_id_of(&stick_id)?)
        .await
        .map_err(shown)?;
    let store = state.store.clone();
    let is_dir = {
        let (store, name) = (store.clone(), name.clone());
        blocking(move || {
            store
                .linked_entries(&stick.stick_id)?
                .into_iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.is_dir)
                .ok_or_else(|| {
                    anyhow::anyhow!("\u{201c}{name}\u{201d} is not sent from where it is.")
                })
        })
        .await?
    };
    let Some(path) = crate::pick::pick_one(&app, &name, is_dir).await? else {
        return Ok(false);
    };
    let files = state.files.clone();
    let data_dir = state.data_dir.clone();
    let handle = app.clone();
    blocking(move || {
        let path = Path::new(&path);
        let kept = crate::pick::kept_here(&handle);
        if let Some(why) = crate::pick::not_kept(kept.as_ref(), path, &name) {
            anyhow::bail!("{why}");
        }
        let found = plan_found(
            files.as_ref(),
            store.as_ref(),
            &stick.stick_id,
            &data_dir,
            &name,
            path,
        )?;
        store.move_linked_entry(&stick.stick_id, &found)?;
        Ok(())
    })
    .await?;
    crate::stick_commands::nudge(&state, &stick.stick_id).await;
    let _ = app.emit("stick-changed", ());
    Ok(true)
}
