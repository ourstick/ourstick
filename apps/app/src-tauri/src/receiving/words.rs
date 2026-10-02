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
             when there is room.",
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
            "This version of the stick cannot be saved on this computer ({what}). It goes on \
             when the sender changes it."
        ),
        Waiting::ThisComputer(what) => {
            format!("Something on this computer failed ({what}); trying again in a moment.")
        }
    }
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
        other => format!(
            "Stopped: {}. Use Try again.",
            ourstick_core::untrusted::escape(&other.to_string())
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
