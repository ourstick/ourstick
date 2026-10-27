//! The window's list of received sticks (step 7b), and of the links it
//! waits for (ADR 0025).

use ourstick_core::received::display_name;
use ourstick_core::StickKind;
use serde::Serialize;
use tauri::State;

use super::{Receivers, Shown};
use crate::state::{stick_hex, AppState};

/// One received stick, as the list shows it.
#[derive(Debug, Serialize)]
pub struct ReceivedEntry {
    id: String,
    name: String,
    /// None while its sender is waited for (ADR 0025): no folder yet.
    folder: Option<String>,
    paused: bool,
    /// A link the app waits for: no files, Forget only.
    waiting: bool,
    /// What is saved under another name than at the sender (ADR 0010).
    saved_as: super::words::SavedAsList,
    /// What the user chose not to get, escaped, and whether files of it
    /// are already here (ADR 0012).
    left_out: Vec<super::choice::LeftOutView>,
    #[serde(flatten)]
    shown: Shown,
}

/// The sticks the window receives, oldest first.
#[tauri::command]
pub async fn received(
    state: State<'_, AppState>,
    receivers: State<'_, Receivers>,
) -> Result<Vec<ReceivedEntry>, String> {
    let store = state.store.clone();
    // From the latest round where there was one; else (before a round has
    // planned, e.g. the sender is away since the start) from what is
    // recorded, read once per call and only then.
    let rounds = receivers.saved_as_lists();
    let joined = super::work(move || {
        let mut sticks = Vec::new();
        for joined in store.joined_sticks()? {
            if joined.kind != StickKind::App {
                continue;
            }
            let saved_as = match rounds.get(&joined.stick_id).cloned() {
                Some(list) => list,
                None => super::words::saved_as_views(&ourstick_core::download::saved_elsewhere(
                    &store.saved_elsewhere(&joined.stick_id)?,
                )),
            };
            let left_out = super::choice::left_out_views(store.as_ref(), &joined.stick_id)?;
            sticks.push((joined, saved_as, left_out));
        }
        Ok((sticks, store.waiting_links()?))
    })
    .await?;
    let (joined, waiting) = joined;
    let received = joined
        .into_iter()
        .map(|(joined, saved_as, left_out)| ReceivedEntry {
            id: stick_hex(&joined.stick_id),
            name: display_name(joined.name.as_deref(), &joined.stick_id),
            folder: Some(joined.output_dir.display().to_string()),
            paused: joined.paused,
            waiting: false,
            saved_as,
            left_out,
            shown: super::device_words::shown_here(receivers.shown(&joined.stick_id)),
        });
    // Then the links waited for (ADR 0025), oldest first.
    let waiting = waiting.into_iter().map(|link| ReceivedEntry {
        id: stick_hex(&link.stick_id),
        name: display_name(None, &link.stick_id),
        folder: None,
        paused: false,
        waiting: true,
        saved_as: super::words::saved_as_views(&[]),
        left_out: Vec::new(),
        shown: super::device_words::shown_here(receivers.shown(&link.stick_id)),
    });
    Ok(received.chain(waiting).collect())
}
