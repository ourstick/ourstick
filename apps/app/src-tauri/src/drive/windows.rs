//! Windows: each stick a folder in `%USERPROFILE%\OurStick`, shown as a
//! drive letter with `subst`, and by its name at the root of File
//! Explorer's pane ([`super::pane`], ADR 0023). The letter shows the
//! system disk's name (no label applies to a `subst` drive; see
//! docs/gui-spec.md). A `subst` letter lasts until the user logs off, so
//! the app gives it again at every start, asking for the letter it had
//! before. Eject takes the letter and the entry away; the folder stays.
//!
//! Which letter shows the stick is found by the drive marker (it holds the
//! StickId), not by reading `subst`'s output, whose code page would garble a
//! non-ASCII folder name.

/// The letters to try, in order: the one the stick had, then `O`, then `Z`
/// down to `D` (`A`–`C` are floppies and the system drive).
fn candidates(preferred: Option<char>) -> Vec<char> {
    let mut letters: Vec<char> = preferred.into_iter().collect();
    for letter in std::iter::once('O').chain(('D'..='Z').rev()) {
        if !letters.contains(&letter) {
            letters.push(letter);
        }
    }
    letters
}

/// The letters in what `subst` prints, one `X:\: => path` per line.
fn subst_letters_in(output: &[u8]) -> Vec<char> {
    output
        .split(|byte| *byte == b'\n')
        .filter_map(|line| match line {
            [letter, b':', ..] if letter.is_ascii_alphabetic() => {
                Some(letter.to_ascii_uppercase() as char)
            }
            _ => None,
        })
        .collect()
}

#[cfg(windows)]
pub mod platform {
    use super::*;
    use crate::drive::pane::platform as pane;
    use crate::drive::{home, no_window, Renamed, NAME};
    use anyhow::{bail, Context, Result};
    use ourstick_core::manifest::DRIVE_MARKER;
    use ourstick_core::OwnedStick;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    fn drive_root(letter: char) -> PathBuf {
        PathBuf::from(format!("{letter}:\\"))
    }

    /// The letters `subst` gives out now. Only the letter of each line is
    /// read (`X:\: => path`): the path is in a code page that would garble
    /// a non-ASCII folder name.
    fn subst_letters() -> Vec<char> {
        let Ok(output) = no_window("subst").output() else {
            return Vec::new();
        };
        subst_letters_in(&output.stdout)
    }

    /// The letters that show the stick at `root`: `subst` letters (never
    /// another drive: an empty card reader would ask for a disk) whose
    /// marker is the same.
    fn letters_showing(root: &Path) -> Vec<char> {
        let Ok(marker) = std::fs::read(root.join(DRIVE_MARKER)) else {
            return Vec::new();
        };
        subst_letters()
            .into_iter()
            .filter(|letter| {
                std::fs::read(drive_root(*letter).join(DRIVE_MARKER))
                    .is_ok_and(|seen| seen == marker)
            })
            .collect()
    }

    fn subst(arguments: &[&std::ffi::OsStr]) -> Result<()> {
        let output = no_window("subst")
            .args(arguments)
            .output()
            .context("cannot run subst")?;
        if !output.status.success() {
            bail!(
                "subst failed: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            );
        }
        Ok(())
    }

    /// Gives `root` the first free letter of [`candidates`].
    fn give_letter(root: &Path, preferred: Option<char>) -> Result<char> {
        for letter in candidates(preferred) {
            if drive_root(letter).exists() {
                continue;
            }
            let drive = format!("{letter}:");
            if subst(&[drive.as_ref(), root.as_os_str()]).is_ok() {
                return Ok(letter);
            }
        }
        bail!("no drive letter is free for the stick")
    }

    /// The marker hidden and a system file, so Explorer does not show it.
    pub fn hide_marker(root: &Path) {
        let marker = root.join(DRIVE_MARKER);
        let _ = no_window("attrib").args(["+h", "+s"]).arg(&marker).output();
    }

    /// Where the app's sticks are: `%USERPROFILE%\OurStick`, a folder each.
    pub fn sticks_home() -> Result<PathBuf> {
        Ok(home()?.join(NAME))
    }

    /// Gives the stick its letter again (the one it had, if free), and
    /// writes its entry in File Explorer's pane (a failure there is logged:
    /// the letter and sharing matter more). The folder itself never moves.
    pub fn mount(stick: &OwnedStick) -> Result<PathBuf> {
        let root = &stick.root_path;
        if letters_showing(root).is_empty() {
            give_letter(root, stick.drive_letter)?;
        }
        forget_seen(root);
        hide_marker(root);
        pane::show_logged(stick);
        Ok(root.clone())
    }

    fn take_letters(root: &Path) -> Result<()> {
        for letter in letters_showing(root) {
            let drive = format!("{letter}:");
            subst(&[drive.as_ref(), "/D".as_ref()])
                .with_context(|| format!("cannot take the letter {letter}: away"))?;
        }
        Ok(())
    }

    /// Takes the stick's letter and its entry in File Explorer's pane
    /// away, the entry even if no letter showed the stick any more (after
    /// a logoff). The folder stays, and so does sharing.
    pub fn eject(stick: &OwnedStick) -> Result<()> {
        let letters = take_letters(&stick.root_path);
        forget_seen(&stick.root_path);
        if let Err(error) = pane::hide(&stick.stick_id) {
            tracing::warn!("{error:#}");
        }
        letters
    }

    /// Renames the stick's folder to `folder` (in the same place): its
    /// letter goes first (`subst` keeps the old path) and comes back after;
    /// a letter that cannot come back is told, not undone. Its entry in
    /// File Explorer's pane is rewritten in place with `name` and the new
    /// folder, or as it was if the folder could not move. Windows refuses
    /// while a file in it is open; the error says to close it.
    pub fn rename(stick: &OwnedStick, folder: &str, name: &str) -> Result<Renamed> {
        let root = &stick.root_path;
        let renamed = crate::drive::renamed_root(root, folder)?;
        let letter = letters_showing(root)
            .first()
            .copied()
            .or(stick.drive_letter);
        take_letters(root)?;
        if &renamed != root {
            if let Err(error) = std::fs::rename(root, &renamed) {
                // The letter comes back where it was, the entry as it was.
                let _ = give_letter(root, letter);
                forget_seen(root);
                pane::show_logged(stick);
                return Err(error).with_context(|| {
                    format!(
                        "cannot rename {}: if a file in it is open, close it and try again",
                        root.display()
                    )
                });
            }
        }
        // Both are tried: the entry must follow the folder even without
        // a letter.
        let no_letter = give_letter(&renamed, letter).err();
        forget_seen(root);
        forget_seen(&renamed);
        let no_entry = pane::show(&stick.stick_id, name, &renamed).err();
        let unplugged = no_letter
            .or(no_entry)
            .map(|error| crate::words::told(&error));
        Ok(Renamed {
            root: renamed,
            unplugged,
        })
    }

    /// What `subst` last showed of each stick's folder: its letter, or none
    /// (ejected), and when. The app asks every few seconds; reading the
    /// marker at that letter answers without starting `subst`, a program
    /// the antivirus looks at each time. A letter only `subst` gave: never
    /// another drive's.
    static SEEN: std::sync::Mutex<Vec<Seen>> = std::sync::Mutex::new(Vec::new());

    struct Seen {
        root: PathBuf,
        letter: Option<char>,
        at: Instant,
    }

    /// How long a stick `subst` showed at no letter is taken as still
    /// without one: a letter given behind the app's back shows this late.
    pub(super) const ASK_SUBST_AGAIN: Duration = Duration::from_secs(30);

    pub fn is_mounted(stick: &OwnedStick) -> bool {
        mounted_at(&stick.root_path, Instant::now())
    }

    /// [`is_mounted`] at `now`.
    pub(super) fn mounted_at(root: &Path, now: Instant) -> bool {
        let mut seen = SEEN.lock().expect("drive letters lock");
        if let Some(index) = seen.iter().position(|seen| seen.root == root) {
            match seen[index].letter {
                Some(letter) => {
                    let marker = std::fs::read(root.join(DRIVE_MARKER));
                    let at_letter = std::fs::read(drive_root(letter).join(DRIVE_MARKER));
                    if marker.is_ok_and(|marker| at_letter.is_ok_and(|at| at == marker)) {
                        return true;
                    }
                }
                None if now.saturating_duration_since(seen[index].at) < ASK_SUBST_AGAIN => {
                    return false;
                }
                None => {}
            }
            seen.remove(index);
        }
        let letter = letters_showing(root).first().copied();
        seen.push(Seen {
            root: root.to_path_buf(),
            letter,
            at: now,
        });
        letter.is_some()
    }

    /// The app gave or took the stick's letter: the next ask runs `subst`.
    fn forget_seen(root: &Path) {
        SEEN.lock()
            .expect("drive letters lock")
            .retain(|seen| seen.root != root);
    }

    /// The letter that shows the stick now, if any.
    pub fn letter_of(root: &Path) -> Option<char> {
        letters_showing(root).first().copied()
    }

    /// Where the file manager should open the stick: its drive letter.
    pub fn shown_at(root: &Path) -> PathBuf {
        letter_of(root).map_or_else(|| root.to_path_buf(), drive_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subst_letters_are_read_whatever_the_paths_code_page() {
        let output = b"O:\\: => C:\\Users\\\xc4\xf6\\OurStick\\F\xfc\r\nz:\\: => D:\\x\r\n\r\n";
        assert_eq!(subst_letters_in(output), ['O', 'Z']);
        assert!(subst_letters_in(b"").is_empty());
    }

    #[test]
    fn the_stick_asks_for_its_old_letter_first() {
        let letters = candidates(Some('S'));
        assert_eq!(&letters[..4], &['S', 'O', 'Z', 'Y']);
        assert_eq!(letters.len(), 23, "D to Z, each once");
        assert!(!letters.contains(&'C'));
        assert_eq!(&candidates(None)[..3], &['O', 'Z', 'Y']);
        assert_eq!(
            candidates(Some('O')).iter().filter(|l| **l == 'O').count(),
            1
        );
    }

    /// A letter taken away behind the app's back, given back, then given
    /// to another stick's folder: the mount check must see each, though it
    /// remembers the letter, and a missing one for a while. It gives out a
    /// real drive letter for a moment, so it runs only by hand:
    /// `cargo test -p ourstick-app -- --ignored letter_taken`.
    #[cfg(windows)]
    #[test]
    #[ignore = "gives out a real drive letter"]
    fn a_letter_taken_away_or_given_to_another_stick_is_seen() {
        use ourstick_core::share::mark_drive;
        use ourstick_core::StickSigningKey;
        use platform::{mounted_at, ASK_SUBST_AGAIN};
        use std::path::{Path, PathBuf};
        use std::process::Command;
        use std::time::Instant;

        /// The test's folders and letter, both removed when dropped.
        struct Place(PathBuf, String);
        impl Drop for Place {
            fn drop(&mut self) {
                let _ = Command::new("subst").args([&self.1, "/D"]).output();
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let letter = candidates(None)
            .into_iter()
            .find(|letter| !PathBuf::from(format!("{letter}:\\")).exists())
            .expect("a free letter");
        let place = Place(
            std::env::temp_dir().join(format!("ourstick-letters-{}", std::process::id())),
            format!("{letter}:"),
        );
        let stick = |byte: u8| {
            let root = place.0.join(format!("stick-{byte}"));
            std::fs::create_dir_all(&root).unwrap();
            let id = StickSigningKey::from_bytes(&[byte; 32]).stick_id();
            mark_drive(&root, &id).unwrap();
            root
        };
        let (first, second) = (stick(1), stick(2));
        let subst = |target: &Path| {
            let done = Command::new("subst").arg(&place.1).arg(target).output();
            assert!(done.unwrap().status.success());
        };
        let take = || {
            let done = Command::new("subst").args([&place.1, "/D"]).output();
            assert!(done.unwrap().status.success());
        };
        let now = Instant::now();
        let later = now + ASK_SUBST_AGAIN;

        subst(&first);
        assert!(mounted_at(&first, now), "found through subst");
        assert!(mounted_at(&first, now), "found at the remembered letter");
        take();
        assert!(!mounted_at(&first, now), "its letter was taken away");

        subst(&first);
        assert!(!mounted_at(&first, now), "no letter, without asking subst");
        assert!(mounted_at(&first, later), "subst asked again");
        take();
        subst(&second);
        assert!(!mounted_at(&first, later), "its letter shows another stick");
        assert!(mounted_at(&second, later));
    }
}
