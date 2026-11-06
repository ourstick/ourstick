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

/// How many times the clipboard is tried, 5 ms apart: another program (the
/// history, a clipboard manager, a remote desktop's sync) may hold it for a
/// moment. clipboard-win's own retries do not wait between tries.
const ATTEMPTS: u32 = 20;

/// Puts `text` on the clipboard, kept by neither its history nor its cloud.
/// Never left there without its marks: if one cannot be set, the clipboard
/// is emptied. Blocking (it waits for the clipboard).
pub fn put_text(text: &str) -> Result<()> {
    let _open = open()?;
    let marked = raw::set_string(text)
        .map_err(|error| anyhow!("{error}"))
        .and_then(|()| mark());
    if marked.is_err() {
        let _ = raw::empty();
    }
    marked
}

fn open() -> Result<Clipboard> {
    let mut tries = 1;
    loop {
        match Clipboard::new() {
            Ok(open) => return Ok(open),
            Err(error) if tries >= ATTEMPTS => return Err(anyhow!("{error}")),
            Err(_) => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
}

/// The three formats beside the text, while the clipboard is open.
fn mark() -> Result<()> {
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
