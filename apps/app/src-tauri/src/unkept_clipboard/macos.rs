//! macOS: the text with the concealed mark beside it
//! (`org.nspasteboard.ConcealedType`, nspasteboard.org), which clipboard
//! history apps and Universal Clipboard tools honour, as for password
//! managers' copies. Both are one pasteboard item, written at once by the
//! system's JavaScript for Automation (`osascript`, about 0.15 s): an app
//! that looks at the clipboard at any moment finds the link with its mark,
//! or nothing, never the link alone. No unsafe code of our own, no new
//! crate. The link goes to the script's input, never onto its command line,
//! which other programs can read. Should the script fail, arboard writes
//! the text and then the mark, in two writes, as before.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use arboard::{Clipboard, SetExtApple};

/// The script; `BOARD` names the pasteboard it writes. The text comes on its
/// input; it says "written" once the item, text and mark, is there.
const SCRIPT: &str = r#"ObjC.import('AppKit');
function run() {
  var input = $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
  var text = $.NSString.alloc.initWithDataEncoding(input, $.NSUTF8StringEncoding);
  var item = $.NSPasteboardItem.alloc.init;
  if (!item.setStringForType($(''), $('org.nspasteboard.ConcealedType'))
      || !item.setStringForType(text, $('public.utf8-plain-text'))) {
    return 'not written';
  }
  var board = BOARD;
  board.clearContents;
  return board.writeObjects($([item])) ? 'written' : 'not written';
}"#;

/// The clipboard.
const GENERAL: &str = "$.NSPasteboard.generalPasteboard";

/// How long the script may take before the two writes are used instead.
const AT_MOST: Duration = Duration::from_secs(5);

/// Puts `text` on the clipboard, marked to be left out of its history.
pub fn put_text(text: &str) -> Result<()> {
    match in_one_write(text, GENERAL) {
        Ok(()) => Ok(()),
        Err(error) => {
            tracing::warn!(%error, "the link goes to the clipboard in two writes");
            in_two_writes(text)
        }
    }
}

/// The text and its mark as one item, written at once on `board`. What the
/// script prints on errors is not kept: it is never needed, and the text
/// must not reach a log.
fn in_one_write(text: &str, board: &str) -> Result<()> {
    let mut script = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", &SCRIPT.replace("BOARD", board)])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    // Closed once written (dropped): the script reads to the end.
    script
        .stdin
        .take()
        .ok_or_else(|| anyhow!("osascript has no input"))?
        .write_all(text.as_bytes())?;
    let started = Instant::now();
    while script.try_wait()?.is_none() {
        if started.elapsed() > AT_MOST {
            let _ = script.kill();
            let _ = script.wait();
            bail!("osascript took over {} s", AT_MOST.as_secs());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = script.wait_with_output()?;
    if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "written" {
        Ok(())
    } else {
        bail!("osascript did not write it ({})", output.status)
    }
}

/// The text, then the mark beside it: what arboard does.
fn in_two_writes(text: &str) -> Result<()> {
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

    /// A pasteboard of the tests' own: the user's clipboard is never
    /// touched, and it needs no desktop session.
    const TEST_BOARD: &str = "$.NSPasteboard.pasteboardWithName($('local.ourstick.test'))";

    /// The test pasteboard's items, the first one's types and its text.
    fn read_back() -> String {
        let script = r#"ObjC.import('AppKit');
function run() {
  var board = $.NSPasteboard.pasteboardWithName($('local.ourstick.test'));
  var items = board.pasteboardItems;
  var item = items.objectAtIndex(0);
  var types = [];
  for (var i = 0; i < item.types.count; i++) types.push(item.types.objectAtIndex(i).js);
  var said = items.count + ' ' + types.join(',') + ' ' + item.stringForType($('public.utf8-plain-text')).js;
  board.releaseGlobally;
  return said;
}"#;
        let read = Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-e", script])
            .output()
            .unwrap();
        String::from_utf8_lossy(&read.stdout).trim().to_owned()
    }

    #[test]
    fn the_link_and_its_mark_are_one_item_written_at_once() {
        in_one_write("https://ourstick.com/s/#TEST.KEY", TEST_BOARD).unwrap();
        assert_eq!(
            read_back(),
            "1 org.nspasteboard.ConcealedType,public.utf8-plain-text \
             https://ourstick.com/s/#TEST.KEY"
        );
    }

    #[test]
    fn a_script_that_cannot_run_is_an_error() {
        assert!(in_one_write("x", "undefined_board").is_err());
    }

    /// On a desktop session only: an ssh session may have no clipboard.
    #[test]
    #[ignore = "needs a macOS desktop session"]
    fn the_text_is_there() {
        put_text("https://ourstick.com/s/#TEST").unwrap();
        let text = Clipboard::new().unwrap().get_text().unwrap();
        assert_eq!(text, "https://ourstick.com/s/#TEST");
    }
}
