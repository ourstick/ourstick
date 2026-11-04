//! Where a stick may be received into, as the window asks: the window
//! names the folder (the native picker's choice, or one OurStick made in
//! Downloads), so these rules hold whatever it names.

use std::path::{Component, Path};

/// Why `folder` is no place to receive into, if it is not: not a full
/// path, the home folder itself, or inside a hidden folder of it (settings
/// and keys live there: `.ssh`, `.config`).
pub fn refused(folder: &Path, home: Option<&Path>) -> Option<&'static str> {
    if !folder.is_absolute() {
        return Some("That is not a full path.");
    }
    let inside = folder.strip_prefix(home?).ok()?;
    match inside.components().next() {
        None => {
            Some("Your whole home folder cannot be a stick's folder. Choose a folder inside it.")
        }
        Some(Component::Normal(first)) if first.to_string_lossy().starts_with('.') => {
            Some("That folder holds private settings. Choose another one.")
        }
        _ => None,
    }
}

/// The user's home folder, if known.
pub fn home() -> Option<std::path::PathBuf> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable)
        .filter(|home| !home.is_empty())
        .map(std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_and_its_hidden_folders_are_refused() {
        let home = Path::new(if cfg!(windows) {
            r"C:\Users\a"
        } else {
            "/home/a"
        });
        let at = |rest: &str| home.join(rest);
        assert!(refused(home, Some(home)).is_some());
        assert!(refused(&at(".ssh"), Some(home)).is_some());
        assert!(refused(&at(".config/autostart"), Some(home)).is_some());
        assert!(refused(Path::new("relative"), Some(home)).is_some());
        assert_eq!(refused(&at("Downloads/Holiday"), Some(home)), None);
        assert_eq!(refused(&at("Photos/.hidden-inside"), Some(home)), None);
        assert_eq!(refused(&at("Downloads"), None), None);
    }
}
