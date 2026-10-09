//! A sharing link already on the clipboard, for "Add a sharing link" to
//! start with: the user copied it from a message, so need not paste it.
//! Only a link is given back, never anything else the clipboard holds; and,
//! as every link, it is neither logged nor put in a message.

use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

/// The clipboard's text, if it is a whole OurStick sharing link.
#[tauri::command]
pub fn link_on_clipboard(app: AppHandle) -> Option<String> {
    let text = app.clipboard().read_text().ok()?;
    link_in(&text)
}

/// `text`, trimmed, if it is a sharing link.
fn link_in(text: &str) -> Option<String> {
    let text = text.trim();
    super::commands::ticket_of(text)
        .ok()
        .map(|_| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_whole_link_is_taken() {
        assert_eq!(link_in(""), None);
        assert_eq!(link_in("hello"), None);
        assert_eq!(link_in("https://ourstick.com/s/#not-a-link"), None);
    }

    #[test]
    fn a_link_copied_with_spaces_around_is_taken_trimmed() {
        let ticket = ourstick_core::Ticket {
            stick_id: ourstick_core::StickSigningKey::from_bytes(&[1; 32]).stick_id(),
            node_id: ourstick_core::NodeId([7; 32]),
            manifest_hint: ourstick_core::ManifestHint {
                seq: 1,
                hash: [0; 32],
            },
            address_hints: Vec::new(),
            stick_key: ourstick_core::cipher::StickKey::from_bytes([2; 32]),
        };
        let link = ticket.to_link().unwrap();
        assert_eq!(link_in(&format!("  {link}\n")), Some(link));
    }
}
