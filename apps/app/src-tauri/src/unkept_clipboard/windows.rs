//! Windows: Win+V's history and its cloud sync keep what is copied; three
//! formats beside the text tell them not to (Microsoft's "Clipboard
//! Formats", the ones password managers set). All in one opening of the
//! clipboard, so nothing sees the text without them. Through clipboard-win:
//! no unsafe code of our own.

use anyhow::{anyhow, Result};
use clipboard_win::{raw, Clipboard};

/// Its presence alone: history, cloud and clipboard monitors leave the
/// content alone.
const EXCLUDE: &str = "ExcludeClipboardContentFromMonitorProcessing";
/// A DWORD 0: not into Win+V's history.
const NO_HISTORY: &str = "CanIncludeInClipboardHistory";
/// A DWORD 0: not to the cloud clipboard.
const NO_CLOUD: &str = "CanUploadToCloudClipboard";

/// Puts `text` on the clipboard, kept by neither its history nor its cloud.
pub fn put_text(text: &str) -> Result<()> {
    // Another program may hold the clipboard for a moment.
    let _open = Clipboard::new_attempts(10).map_err(|error| anyhow!("{error}"))?;
    raw::set_string(text).map_err(|error| anyhow!("{error}"))?;
    for (name, data) in [
        (EXCLUDE, [0u8; 4]),
        (NO_HISTORY, [0u8; 4]),
        (NO_CLOUD, [0u8; 4]),
    ] {
        let format =
            raw::register_format(name).ok_or_else(|| anyhow!("no clipboard format {name}"))?;
        raw::set_without_clear(format.get(), &data).map_err(|error| anyhow!("{error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a desktop session only: an ssh session has no clipboard.
    #[test]
    #[ignore = "needs a Windows desktop session"]
    fn the_text_is_there_with_the_marks() {
        put_text("https://ourstick.com/s/#TEST").unwrap();
        let _open = Clipboard::new_attempts(10).unwrap();
        let mut text = Vec::new();
        raw::get_string(&mut text).unwrap();
        assert_eq!(text, b"https://ourstick.com/s/#TEST");
        for name in [EXCLUDE, NO_HISTORY, NO_CLOUD] {
            let format = raw::register_format(name).unwrap();
            assert!(raw::is_format_avail(format.get()), "{name}");
        }
    }
}
