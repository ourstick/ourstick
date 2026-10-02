//! What a received stick is doing, in words for everyone (no jargon but
//! "sharing link"): one place for every message the window shows about it.

use ourstick_core::download::{DownloadError, Waiting};
use ourstick_core::record::RecordError;

/// A size for people, written as the window writes it (`src/lib/format.ts`
/// `size`): `12 GB`, `1.5 GB`, `340 MB`, `4 KB`.
fn size(bytes: u64) -> String {
    let bytes = bytes as f64;
    if bytes >= 1e9 {
        let gigabytes = bytes / 1e9;
        if gigabytes.fract() == 0.0 {
            format!("{gigabytes} GB")
        } else {
            format!("{gigabytes:.1} GB")
        }
    } else if bytes >= 1e6 || bytes == 0.0 {
        format!("{} MB", (bytes / 1e6).round())
    } else {
        format!("{} KB", (bytes / 1e3).round().max(1.0))
    }
}

/// One name saved under another here (ADR 0010), as the window lists it:
/// both escaped (they are the sender's text).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SavedAsView {
    path: String,
    saved_as: String,
    /// The files it holds (1 for a file, more for a folder).
    files: usize,
}

/// What the window gets of the names saved under another: the first
/// [`SAVED_AS_LISTED`] lines (the window shows 10, then all of these), how
/// many more, and how many files in all.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SavedAsList {
    lines: Vec<SavedAsView>,
    more: usize,
    files: usize,
}

/// The most lines sent to the window at once.
pub const SAVED_AS_LISTED: usize = 200;

/// The window's list of what is saved under another name.
pub fn saved_as_views(lines: &[ourstick_core::download::SavedAs]) -> SavedAsList {
    SavedAsList {
        lines: lines
            .iter()
            .take(SAVED_AS_LISTED)
            .map(|line| SavedAsView {
                path: ourstick_core::untrusted::escape(&line.path),
                saved_as: ourstick_core::untrusted::escape(&line.saved_as),
                files: line.files,
            })
            .collect(),
        more: lines.len().saturating_sub(SAVED_AS_LISTED),
        files: lines.iter().map(|line| line.files).sum(),
    }
}

/// Why a received stick waits, and that it will try again by itself.
pub fn waiting_words(why: &Waiting) -> String {
    match why {
        Waiting::Sender(_) => {
            "The sender's computer is off, or OurStick is not open there. It goes on by itself \
             when they are back."
                .to_owned()
        }
        Waiting::NoLongerShared => {
            "The sender no longer shares this stick with this sharing link. If they gave it a new \
             one, add that one."
                .to_owned()
        }
        Waiting::Room { needed, available } => format!(
            "Not enough room on this computer: it needs up to {}, and {} are left. It goes on \
             when there is room, or leave something out with Choose what to get… (in ⋯).",
            size(*needed),
            size(*available)
        ),
        Waiting::DiskFull => {
            "The disk it is saved to is full. It goes on when there is room.".to_owned()
        }
        Waiting::Folder => {
            "Its folder is not there (is its disk unplugged?). It goes on when it is back."
                .to_owned()
        }
        Waiting::KeepsChanging => {
            "The sender's stick keeps changing while it comes; trying again in a moment.".to_owned()
        }
        Waiting::Damaged(_) => {
            "What came from the sender was damaged; trying again in a few minutes.".to_owned()
        }
        Waiting::CannotTake(what) => format!(
            "This version of the stick cannot be saved in this folder ({what}). It goes on when \
             the sender or the folder changes."
        ),
        Waiting::ThisComputer(what) => {
            format!("Something on this computer failed ({what}); trying again in a moment.")
        }
    }
}

/// Any error of the receiver's work, as the window says it: plain words for
/// the user (the details go to the log). Sentences OurStick wrote itself
/// are kept; what came from the keyring, the store, a lock or the disk is
/// said in words for everyone.
pub fn said(error: &anyhow::Error) -> String {
    use ourstick_core::device::DeviceError;
    use ourstick_core::received::ReceivedError;
    // The keyring and the records: the same words everywhere.
    if let Some(words) = crate::words::known(error) {
        return words;
    }
    tracing::warn!(error = %format!("{error:#}"), "told in the window in words");
    const DISK: &str = "Something on this computer failed (a folder or a file). Try again.";
    if let Some(device) = error.downcast_ref::<DeviceError>() {
        return match device {
            DeviceError::Lock(locked) => super::lock_words(locked),
            DeviceError::Io { .. } | DeviceError::NoHome(_) => DISK.to_owned(),
            // A key missing or not the right one: its own words say what to do.
            other => sentence(&other.to_string()),
        };
    }
    if let Some(download) = error.downcast_ref::<DownloadError>() {
        return match download {
            DownloadError::BeingForgotten => {
                "This stick is still being forgotten. Restart OurStick, then add it again."
                    .to_owned()
            }
            other => stopped_words(other),
        };
    }
    if let Some(locked) = error.downcast_ref::<ourstick_core::locks::LockError>() {
        return super::lock_words(locked);
    }
    if let Some(received) = error.downcast_ref::<ReceivedError>() {
        return match received {
            ReceivedError::Lock(locked) => super::lock_words(locked),
            ReceivedError::Io { .. } => DISK.to_owned(),
            other => sentence(&other.to_string()),
        };
    }
    if error
        .downcast_ref::<ourstick_core::places::PlaceError>()
        .is_some()
    {
        return sentence(&ourstick_core::untrusted::escape(&error.to_string()));
    }
    if error.downcast_ref::<std::io::Error>().is_some() {
        return DISK.to_owned();
    }
    sentence(&error.to_string())
}

/// `text` as a sentence: a capital first letter and a full stop.
fn sentence(text: &str) -> String {
    let mut characters = text.chars();
    let mut said: String = match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => return String::new(),
    };
    if !said.ends_with(['.', '!', '?']) {
        said.push('.');
    }
    said
}

/// Why a received stick stopped, and what to do.
pub fn stopped_words(error: &DownloadError) -> String {
    match error {
        DownloadError::Record(
            RecordError::OlderThanTicket { .. }
            | RecordError::NotNewer { .. }
            | RecordError::ConflictsWithTicket { .. }
            | RecordError::ConflictsWithCurrent { .. },
        ) => "This stick went back to an older version, which OurStick does not take. Ask the \
             sender for a new sharing link."
            .to_owned(),
        DownloadError::Record(_) => {
            "What came does not belong to this sharing link. Ask the sender for a new one."
                .to_owned()
        }
        DownloadError::ReceivedByApp(_) | DownloadError::BeingForgotten => {
            "Another OurStick program on this computer has this stick. Close it, then use Try again."
                .to_owned()
        }
        DownloadError::Io { .. } => {
            "Saving it on this computer failed. Use Try again.".to_owned()
        }
        // A shortcut (symbolic link) in the way of the folder's own files.
        DownloadError::UnsafePath { path, .. } => format!(
            "\u{201c}{}\u{201d} cannot be saved in this folder: a shortcut to another place is in \
             its way. Move the shortcut out of the folder, then use Try again.",
            ourstick_core::untrusted::escape(path)
        ),
        DownloadError::Manifest(_) | DownloadError::Cipher(_) => {
            "What came from the sender could not be read. Use Try again; if it goes on, ask the \
             sender for a new sharing link."
                .to_owned()
        }
        other => {
            tracing::warn!(error = %other, "a received stick stopped");
            "It stopped. Use Try again.".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_said_in_words_for_everyone() {
        let locked = || ourstick_core::SecretError::AccessDenied {
            store: "keyring".to_owned(),
            reason: "locked".to_owned(),
        };
        // As the keyring's own error, and as opening it gives it (the
        // device's): both in words.
        let opening = anyhow::Error::from(ourstick_core::device::DeviceError::from(locked()));
        assert!(
            said(&opening).starts_with("OurStick could not reach the passwords"),
            "{}",
            said(&opening)
        );
        let keyring = anyhow::Error::from(locked());
        assert!(said(&keyring).starts_with("OurStick could not reach the passwords"));
        let forgotten = anyhow::Error::from(DownloadError::BeingForgotten);
        assert!(said(&forgotten).contains("Restart OurStick"));
        let store = anyhow::Error::from(ourstick_core::StoreError::StickNotFound);
        assert!(said(&store).starts_with("OurStick's own records"));
        let disk = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::Other));
        assert!(said(&disk).starts_with("Something on this computer"));
        let ours = anyhow::anyhow!("this stick is not received here any more");
        assert_eq!(said(&ours), "This stick is not received here any more.");
        let refused = stopped_words(&DownloadError::UnsafePath {
            path: "Photos/a.png".to_owned(),
            reason: "it would pass through a symbolic link",
        });
        assert_eq!(
            refused,
            "\u{201c}Photos/a.png\u{201d} cannot be saved in this folder: a shortcut to another \
             place is in its way. Move the shortcut out of the folder, then use Try again."
        );
        let stopped = stopped_words(&DownloadError::Cancelled);
        assert_eq!(stopped, "It stopped. Use Try again.");
    }

    #[test]
    fn saved_as_lines_are_escaped() {
        let line = |path: &str| ourstick_core::download::SavedAs {
            path: path.to_owned(),
            saved_as: format!("{path}_"),
            files: 2,
        };
        let views = saved_as_views(&[line("a\u{1b}?.jpg")]);
        assert_eq!(views.lines[0].path, "a\\u{1b}?.jpg");
        assert_eq!(views.lines[0].saved_as, "a\\u{1b}?.jpg_");
        // At most SAVED_AS_LISTED lines go to the window; the rest counted.
        let many: Vec<_> = (0..SAVED_AS_LISTED + 5)
            .map(|n| line(&n.to_string()))
            .collect();
        let views = saved_as_views(&many);
        assert_eq!(views.lines.len(), SAVED_AS_LISTED);
        assert_eq!(views.more, 5);
        assert_eq!(views.files, 2 * (SAVED_AS_LISTED + 5));
    }

    #[test]
    fn sizes_are_written_as_the_window_writes_them() {
        assert_eq!(size(12_000_000_000), "12 GB");
        assert_eq!(size(1_500_000_000), "1.5 GB");
        assert_eq!(size(340_000_000), "340 MB");
        assert_eq!(size(0), "0 MB");
        assert_eq!(size(10), "1 KB");
    }

    #[test]
    fn every_wait_says_it_goes_on_and_never_needs_0_b() {
        for why in [
            Waiting::Sender("x".to_owned()),
            Waiting::NoLongerShared,
            Waiting::Room {
                needed: 12 << 30,
                available: 3 << 30,
            },
            Waiting::DiskFull,
            Waiting::Folder,
            Waiting::KeepsChanging,
            Waiting::Damaged("x".to_owned()),
            Waiting::CannotTake("a name too long".to_owned()),
            Waiting::ThisComputer("x".to_owned()),
        ] {
            let words = waiting_words(&why);
            assert!(!words.contains(" 0 "), "{words}");
            assert!(!words.is_empty());
        }
        assert!(waiting_words(&Waiting::Room {
            needed: 12 << 30,
            available: 3 << 30
        })
        .contains("12"));
    }
}
