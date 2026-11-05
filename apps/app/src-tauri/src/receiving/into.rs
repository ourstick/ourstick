//! Where a stick may be received into, as the window asks: the window
//! names the folder (the native picker's choice, or one OurStick made in
//! Downloads), so these rules hold whatever it names.

use std::path::{Component, Path};

use ourstick_core::places::comparable;

/// Why `folder` is no place to receive into, if it is not: not a full
/// path, the home folder itself, or inside a folder of it that holds
/// settings and keys (a hidden one: `.ssh`, `.config`; `AppData` on
/// Windows, `Library` on macOS). Compared as the disk sees the paths:
/// links resolved, `..` folded, letter case ignored where the disk does.
pub fn refused(folder: &Path, home: Option<&Path>) -> Option<&'static str> {
    if !folder.is_absolute() {
        return Some("That is not a full path.");
    }
    let (folder, home) = (comparable(folder), comparable(home?));
    let inside = folder.strip_prefix(&home).ok()?;
    match inside.components().next() {
        None => {
            Some("Your whole home folder cannot be a stick's folder. Choose a folder inside it.")
        }
        Some(Component::Normal(first)) if holds_settings(&first.to_string_lossy()) => {
            Some("That folder holds private settings. Choose another one.")
        }
        _ => None,
    }
}

/// A first folder of home where programs keep their settings.
fn holds_settings(first: &str) -> bool {
    first.starts_with('.')
        || (cfg!(windows) && first.eq_ignore_ascii_case("appdata"))
        || (cfg!(target_os = "macos") && first.eq_ignore_ascii_case("library"))
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
        // As the disk sees it: `..` folded.
        assert!(refused(&at("Downloads/../.ssh"), Some(home)).is_some());
        assert!(refused(&at("Downloads/.."), Some(home)).is_some());
        if cfg!(windows) {
            assert!(refused(&at(r"AppData\Roaming\x"), Some(home)).is_some());
        }
        if cfg!(target_os = "macos") {
            assert!(refused(&at("Library/LaunchAgents"), Some(home)).is_some());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_hidden_folder_is_refused() {
        let home = std::env::temp_dir().join(format!("ourstick-into-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".config")).unwrap();
        std::os::unix::fs::symlink(home.join(".config"), home.join("Settings")).unwrap();
        let refused_link = refused(&home.join("Settings/autostart"), Some(&home));
        let _ = std::fs::remove_dir_all(&home);
        assert!(refused_link.is_some());
    }
}
