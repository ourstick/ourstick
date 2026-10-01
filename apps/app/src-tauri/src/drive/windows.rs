//! Windows: `%USERPROFILE%\OurStick`, shown as a drive letter with `subst`.
//! A `subst` letter lasts until the user logs off, so the app gives it again
//! at every start, asking for the letter it had before. Eject takes the
//! letter away; the folder stays.
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
    use ourstick_core::manifest::ignore::is_ignored;
    use ourstick_core::manifest::DRIVE_MARKER;
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

    /// Makes `%USERPROFILE%\OurStick` for a new stick and gives it a letter.
    /// An existing folder is used only if it is empty (system files aside)
    /// or already a stick's; anything else is the user's and is left alone.
    pub fn create() -> Result<PathBuf> {
        let root = home()?.join(NAME);
        if root.exists() {
            let has_own_files = std::fs::read_dir(&root)
                .with_context(|| format!("cannot open {}", root.display()))?
                .any(|entry| {
                    entry.is_ok_and(|entry| !entry.file_name().to_str().is_some_and(is_ignored))
                });
            if has_own_files && !folder_is_there(&root) {
                bail!(
                    "{} already exists and holds files. OurStick will not take it over; \
                     move it away and try again.",
                    root.display()
                );
            }
        } else {
            std::fs::create_dir(&root)
                .with_context(|| format!("cannot create {}", root.display()))?;
        }
        give_letter(&root, None)?;
        Ok(root)
    }

    /// Gives the stick its letter again (the one it had, if free). The folder
    /// itself never moves.
    pub fn mount(root: &Path, preferred: Option<char>) -> Result<PathBuf> {
        if letters_showing(root).is_empty() {
            give_letter(root, preferred)?;
        }
        Ok(root.to_path_buf())
    }

    /// Takes the stick's letter away. The folder stays, and so does sharing.
    pub fn eject(root: &Path) -> Result<()> {
        for letter in letters_showing(root) {
            let drive = format!("{letter}:");
            subst(&[drive.as_ref(), "/D".as_ref()])
                .with_context(|| format!("cannot take the letter {letter}: away"))?;
        }
        Ok(())
    }

    pub fn is_mounted(root: &Path) -> bool {
        !letters_showing(root).is_empty()
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
