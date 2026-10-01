//! Windows: each stick a folder in `%USERPROFILE%\OurStick`, shown as a
//! drive letter with `subst`, named after the stick in Explorer (a label
//! for that letter in the user's registry; `subst` has none of its own). A
//! `subst` letter lasts until the user logs off, so the app gives it again
//! at every start, asking for the letter it had before. Eject takes the
//! letter and its label away; the folder stays.
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

#[cfg(windows)]
pub mod platform {
    use super::*;
    use crate::drive::{folder_is_there, home, NAME};
    use anyhow::{bail, Context, Result};
    use ourstick_core::manifest::DRIVE_MARKER;
    use ourstick_core::OwnedStick;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn drive_root(letter: char) -> PathBuf {
        PathBuf::from(format!("{letter}:\\"))
    }

    /// The letters that show the stick at `root`: the same marker is there.
    fn letters_showing(root: &Path) -> Vec<char> {
        let Ok(marker) = std::fs::read(root.join(DRIVE_MARKER)) else {
            return Vec::new();
        };
        ('A'..='Z')
            .filter(|letter| {
                std::fs::read(drive_root(*letter).join(DRIVE_MARKER))
                    .is_ok_and(|seen| seen == marker)
            })
            .collect()
    }

    fn subst(arguments: &[&std::ffi::OsStr]) -> Result<()> {
        let output = Command::new("subst")
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

    /// Where a letter's label lives in the user's registry.
    fn label_key(letter: char) -> String {
        format!(
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\DriveIcons\{letter}\DefaultLabel"
        )
    }

    fn reg(arguments: &[&str]) -> Result<()> {
        let status = Command::new("reg")
            .args(arguments)
            .output()
            .context("cannot run reg")?
            .status;
        if !status.success() {
            bail!("reg failed");
        }
        Ok(())
    }

    /// Names `letter` `name` in Explorer. Best effort: without it the
    /// letter shows the system disk's name.
    fn set_label(letter: char, name: &str) {
        let key = label_key(letter);
        if let Err(error) = reg(&["add", &key, "/ve", "/d", name, "/f"]) {
            tracing::warn!(%error, "the stick's letter keeps the disk's name");
        }
    }

    /// Takes our label off `letter`, so that another drive given that
    /// letter later does not show a stick's name.
    fn remove_label(letter: char) {
        let key =
            format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\DriveIcons\{letter}");
        let _ = reg(&["delete", &key, "/f"]);
    }

    /// The marker hidden and a system file, so Explorer does not show it.
    pub fn hide_marker(root: &Path) {
        let marker = root.join(DRIVE_MARKER);
        let _ = Command::new("attrib")
            .args(["+h", "+s"])
            .arg(&marker)
            .output();
    }

    /// Where the app's sticks are: `%USERPROFILE%\OurStick`, a folder each.
    pub fn sticks_home() -> Result<PathBuf> {
        Ok(home()?.join(NAME))
    }

    /// Makes the folder of a new stick, `%USERPROFILE%\OurStick\<folder>`,
    /// and gives it a letter named `name`. The folder must be new (the
    /// caller picks a free name); the first stick of an older version,
    /// `%USERPROFILE%\OurStick` itself, must have moved into it first.
    pub fn create(folder: &str, name: &str) -> Result<(PathBuf, Option<PathBuf>)> {
        let home = sticks_home()?;
        if folder_is_there(&home) {
            bail!(
                "the first stick is still {}; it moves first",
                home.display()
            );
        }
        std::fs::create_dir_all(&home)
            .with_context(|| format!("cannot create {}", home.display()))?;
        let root = home.join(folder);
        std::fs::create_dir(&root).with_context(|| format!("cannot create {}", root.display()))?;
        let letter = give_letter(&root, None)?;
        set_label(letter, name);
        Ok((root, None))
    }

    /// Gives the stick its letter again (the one it had, if free), named
    /// after it. The folder itself never moves.
    pub fn mount(stick: &OwnedStick) -> Result<PathBuf> {
        let root = &stick.root_path;
        let letter = match letters_showing(root).first() {
            Some(letter) => *letter,
            None => give_letter(root, stick.drive_letter)?,
        };
        set_label(letter, &stick.name);
        hide_marker(root);
        Ok(root.clone())
    }

    /// Takes the stick's letter and its label away. The folder stays, and
    /// so does sharing.
    pub fn eject(stick: &OwnedStick) -> Result<()> {
        for letter in letters_showing(&stick.root_path) {
            let drive = format!("{letter}:");
            subst(&[drive.as_ref(), "/D".as_ref()])
                .with_context(|| format!("cannot take the letter {letter}: away"))?;
            remove_label(letter);
        }
        Ok(())
    }

    pub fn is_mounted(stick: &OwnedStick) -> bool {
        !letters_showing(&stick.root_path).is_empty()
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
}
