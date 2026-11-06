//! macOS: the text with the concealed mark beside it
//! (`org.nspasteboard.ConcealedType`, nspasteboard.org), which clipboard
//! history apps and Universal Clipboard tools honour, as for password
//! managers' copies. Through arboard: no unsafe code of our own.

use anyhow::{anyhow, Result};
use arboard::{Clipboard, SetExtApple};

/// Puts `text` on the clipboard, marked to be left out of its history.
pub fn put_text(text: &str) -> Result<()> {
    let mut clipboard = Clipboard::new().map_err(|error| anyhow!("{error}"))?;
    clipboard
        .set()
        .exclude_from_history()
        .text(text)
        .map_err(|error| anyhow!("{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a desktop session only: an ssh session may have no pasteboard.
    #[test]
    #[ignore = "needs a macOS desktop session"]
    fn the_text_is_there() {
        put_text("https://ourstick.com/s/#TEST").unwrap();
        let text = Clipboard::new().unwrap().get_text().unwrap();
        assert_eq!(text, "https://ourstick.com/s/#TEST");
    }
}
