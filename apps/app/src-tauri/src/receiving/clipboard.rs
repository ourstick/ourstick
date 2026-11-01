//! A sharing link already on the clipboard, for Receive to start with: the
//! user copied it from a message, so need not paste it. Only a link is
//! given back, never anything else the clipboard holds; and, as every link,
//! it is neither logged nor put in a message.
//!
//! The stick of the last link read is remembered, in memory only and as a
//! hash (ADR 0030 §4), so that coming back to the window opens Receive only
//! for a link not seen before.

use std::sync::Mutex;

use ourstick_core::received::check_link;
use ourstick_core::{StickId, StickKind};
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::state::AppState;

/// The clipboard's text, if it is a whole OurStick sharing link.
#[tauri::command]
pub fn link_on_clipboard(app: AppHandle) -> Option<String> {
    read(&app)
}

/// Reads the clipboard; a link found is remembered as seen.
fn read<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let text = app.clipboard().read_text().ok()?;
    let link = link_in(&text)?;
    if let Ok(ticket) = super::commands::ticket_of(&link) {
        app.state::<LastSeen>().saw(&ticket.stick_id);
    }
    Some(link)
}

/// The window came back to Home after it had lost focus (ADR 0030 §4):
/// whether the clipboard holds a link not seen before, of a stick this
/// computer neither sends nor receives. Then Receive is to open, and asks
/// before it looks (§2). Not on macOS, which may ask the user about each
/// such read; not on a phone.
#[tauri::command]
pub async fn new_link_on_clipboard(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    if cfg!(any(target_os = "macos", mobile)) {
        return Ok(false);
    }
    let seen = app.state::<LastSeen>();
    let before = seen.last();
    let Some(link) = read(&app) else {
        return Ok(false);
    };
    let Ok(ticket) = super::commands::ticket_of(&link) else {
        return Ok(false);
    };
    if before == seen.last() {
        return Ok(false);
    }
    let store = state.store.clone();
    let new = super::work(move || {
        Ok(check_link(store.as_ref(), &ticket.stick_id, StickKind::App).is_ok())
    })
    .await?;
    if new {
        app.state::<super::opened::OpenedLink>().asked();
    }
    Ok(new)
}

/// The hash of the stick of the last link read from the clipboard.
#[derive(Default)]
pub struct LastSeen(Mutex<Option<blake3::Hash>>);

impl LastSeen {
    fn saw(&self, stick_id: &StickId) {
        let hash = blake3::hash(crate::state::stick_hex(stick_id).as_bytes());
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(hash);
    }

    fn last(&self) -> Option<blake3::Hash> {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// `text`, trimmed, if it is a sharing link.
pub(super) fn link_in(text: &str) -> Option<String> {
    let text = text.trim();
    super::commands::ticket_of(text)
        .ok()
        .map(|_| text.to_owned())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A whole sharing link, with made-up keys.
    pub fn a_link() -> String {
        link_of(1)
    }

    fn link_of(stick: u8) -> String {
        let ticket = ourstick_core::Ticket {
            stick_id: ourstick_core::StickSigningKey::from_bytes(&[stick; 32]).stick_id(),
            node_id: ourstick_core::NodeId([7; 32]),
            manifest_hint: ourstick_core::ManifestHint {
                seq: 1,
                hash: [0; 32],
            },
            address_hints: Vec::new(),
            stick_key: ourstick_core::cipher::StickKey::from_bytes([2; 32]),
        };
        ticket.to_link().unwrap()
    }

    fn stick_of(link: &str) -> StickId {
        super::super::commands::ticket_of(link).unwrap().stick_id
    }

    #[test]
    fn only_a_whole_link_is_taken() {
        assert_eq!(link_in(""), None);
        assert_eq!(link_in("hello"), None);
        assert_eq!(link_in("https://ourstick.com/s/#not-a-link"), None);
    }

    #[test]
    fn a_link_copied_with_spaces_around_is_taken_trimmed() {
        let link = a_link();
        assert_eq!(link_in(&format!("  {link}\n")), Some(link));
    }

    #[test]
    fn the_same_stick_is_the_same_whatever_follows_the_link() {
        let seen = LastSeen::default();
        assert_eq!(seen.last(), None);
        let link = a_link();
        seen.saw(&stick_of(&link));
        let first = seen.last();
        assert!(first.is_some());
        seen.saw(&stick_of(&format!("{link}.")));
        assert_eq!(seen.last(), first);
        seen.saw(&stick_of(&link_of(9)));
        assert_ne!(seen.last(), first);
    }
}
