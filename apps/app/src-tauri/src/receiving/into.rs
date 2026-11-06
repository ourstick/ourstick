//! Where a stick may be received into, as the window asks: the window
//! names the folder (the native picker's choice, or one OurStick made in
//! Downloads), so these rules hold whatever it names.

use std::path::{Component, Path, PathBuf};

use ourstick_core::places::{comparable, comparable_text};

const NOT_FULL: &str = "That is not a full path.";
const HOME: &str = "Your whole home folder cannot be a stick's folder. Choose a folder inside it.";
const SETTINGS: &str = "That folder holds private settings. Choose another one.";

/// Why `folder` is no place for a received stick's folder, if it is not:
/// not a full path (or one with `.` or `..` in it), the home folder itself,
/// or inside a folder of it that holds settings and keys (a hidden one:
/// `.ssh`, `.config`; `AppData` on Windows, `Library` on macOS, but for
/// iCloud Drive's and the cloud storage apps' folders there). Checked as
/// the path is written and as the disk sees it (links resolved, letter case
/// ignored where the disk does): a link from a settings folder to outside
/// and a link into one are both refused. Blocking: it reads the disk.
pub fn refused(folder: &Path, home: Option<&Path>) -> Option<&'static str> {
    let dots = folder
        .components()
        .any(|part| matches!(part, Component::CurDir | Component::ParentDir));
    if !folder.is_absolute() || dots {
        return Some(NOT_FULL);
    }
    let home = home?;
    let fold = cfg!(any(target_os = "macos", windows));
    let written = |path: &Path| PathBuf::from(comparable_text(&path.to_string_lossy(), fold));
    [
        (written(folder), written(home)),
        (comparable(folder), comparable(home)),
    ]
    .iter()
    .find_map(|(folder, home)| refused_inside(folder, home))
}

/// [`refused`] for `folder` and `home` in one form.
fn refused_inside(folder: &Path, home: &Path) -> Option<&'static str> {
    let inside = folder.strip_prefix(home).ok()?;
    if inside.as_os_str().is_empty() {
        Some(HOME)
    } else if holds_settings(inside) {
        Some(SETTINGS)
    } else {
        None
    }
}

/// Whether `inside` (a path under home) is in a folder of home where
/// programs keep their settings.
fn holds_settings(inside: &Path) -> bool {
    let mut parts = inside.components().filter_map(|part| match part {
        Component::Normal(name) => Some(name.to_string_lossy().to_lowercase()),
        _ => None,
    });
    let Some(first) = parts.next() else {
        return false;
    };
    if first.starts_with('.') {
        return true;
    }
    if cfg!(windows) && first == "appdata" {
        return true;
    }
    if cfg!(target_os = "macos") && first == "library" {
        // iCloud Drive (`Mobile Documents`) and Dropbox, OneDrive, Google
        // Drive and the like (`CloudStorage`) keep the user's files there.
        let second = parts.next();
        return !matches!(second.as_deref(), Some("mobile documents" | "cloudstorage"));
    }
    false
}

/// [`refused`], off the async runtime (a network share asleep can make
/// reading the disk take long): `Err` says why.
pub async fn check(folder: PathBuf) -> Result<(), String> {
    let verdict = tauri::async_runtime::spawn_blocking(move || refused(&folder, home().as_deref()))
        .await
        .map_err(|error| error.to_string())?;
    verdict.map_or(Ok(()), |why| Err(why.to_owned()))
}

/// The user's home folder, if known.
pub fn home() -> Option<PathBuf> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(variable)
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
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
        assert_eq!(refused(home, Some(home)), Some(HOME));
        assert_eq!(refused(&at(".ssh"), Some(home)), Some(SETTINGS));
        assert!(refused(&at(".config/autostart"), Some(home)).is_some());
        assert_eq!(refused(Path::new("relative"), Some(home)), Some(NOT_FULL));
        assert_eq!(refused(&at("Downloads/Holiday"), Some(home)), None);
        assert_eq!(refused(&at("Photos/.hidden-inside"), Some(home)), None);
        assert_eq!(refused(&at("Downloads"), None), None);
        // `.` and `..` are refused whatever they lead to.
        assert_eq!(
            refused(&at("Downloads/../.ssh"), Some(home)),
            Some(NOT_FULL)
        );
        assert_eq!(refused(&at("Downloads/.."), Some(home)), Some(NOT_FULL));
        // A `.` inside a path is the same folder (std drops it).
        assert_eq!(refused(&at("Downloads/./x"), Some(home)), None);
        if cfg!(windows) {
            assert!(refused(&at(r"AppData\Roaming\x"), Some(home)).is_some());
            assert!(refused(&at(r"APPDATA\x"), Some(home)).is_some());
        }
        if cfg!(target_os = "macos") {
            assert!(refused(&at("Library/LaunchAgents"), Some(home)).is_some());
            assert!(refused(&at("Library"), Some(home)).is_some());
            let icloud = "Library/Mobile Documents/com~apple~CloudDocs/Holiday";
            assert_eq!(refused(&at(icloud), Some(home)), None);
            assert_eq!(
                refused(&at("Library/CloudStorage/Dropbox/x"), Some(home)),
                None
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn links_into_and_out_of_a_settings_folder_are_both_refused() {
        let home = std::env::temp_dir().join(format!("ourstick-into-{}", std::process::id()));
        let outside = home.with_extension("outside");
        std::fs::create_dir_all(home.join(".config")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        // A link to a settings folder, under a name that is not hidden.
        std::os::unix::fs::symlink(home.join(".config"), home.join("Settings")).unwrap();
        // A settings folder that is itself a link to outside home.
        std::os::unix::fs::symlink(&outside, home.join(".ssh")).unwrap();
        let into_settings = refused(&home.join("Settings/autostart"), Some(&home));
        let settings_out = refused(&home.join(".ssh/x"), Some(&home));
        let plain = refused(&home.join("Downloads/x"), Some(&home));
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&outside);
        assert_eq!(into_settings, Some(SETTINGS));
        assert_eq!(settings_out, Some(SETTINGS));
        assert_eq!(plain, None);
    }
}
