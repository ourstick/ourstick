//! Windows: each mounted stick an entry at the root of File Explorer's
//! navigation pane, under the stick's name and OurStick's icon, opening
//! its folder (ADR 0023). A `subst` letter cannot carry a name; this does.
//!
//! The entry is Microsoft's per-user shell folder for a cloud provider's
//! folder (`HKCU`, no administrator rights), written with one `.reg` file
//! per change. What OurStick put there is listed under
//! `HKCU\Software\OurStick\NavigationPane\<data folder>`, written before an
//! entry and removed after it, so that a crash never leaves an entry no
//! list names, and two data folders never clean up each other's entries.

use std::path::Path;

use ourstick_core::StickId;

const CLASSES: &str = r"HKEY_CURRENT_USER\Software\Classes\CLSID";
/// Where 32-bit programs look for a class: their Open and Save dialogs
/// show the entry too, not a blank item.
const CLASSES_32: &str = r"HKEY_CURRENT_USER\Software\Classes\WOW6432Node\CLSID";
const EXPLORER: &str = r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer";
/// The lists, one subkey per data folder.
pub const LISTS: &str = r"HKEY_CURRENT_USER\Software\OurStick\NavigationPane";
/// shell32's folder shortcut: a shell folder that shows a target folder.
const FOLDER_SHORTCUT: &str = "{0E5AAE11-A475-4c5b-AB00-C66DE400274E}";
const SHELL32: &str = r"%SystemRoot%\system32\shell32.dll";

/// The class id of a stick's entry: derived from its id, never stored, so
/// the same stick has the same entry at every start.
pub fn class_id(stick_id: &StickId) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"OurStick navigation pane entry");
    hasher.update(&stick_id.to_bytes());
    let hex: String = hasher.finalize().as_bytes()[..16]
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect();
    format!(
        "{{{}-{}-{}-{}-{}}}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The list subkey of the data folder `data_dir`.
pub fn list_name(data_dir: &Path) -> String {
    let hash = blake3::hash(data_dir.to_string_lossy().as_bytes());
    hash.as_bytes()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What an entry shows.
pub struct Entry<'a> {
    pub stick_id: &'a StickId,
    pub name: &'a str,
    pub folder: &'a Path,
    /// The program whose first icon the entry shows: the app itself.
    pub icon: &'a Path,
}

/// A `.reg` string: in quotes, with `\` and `"` escaped. A stick's name
/// holds no control character (`stick_names`), so no line break.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', r"\\").replace('"', "\\\""))
}

/// A `REG_EXPAND_SZ` value: its UTF-16LE bytes and the final NUL, in hex.
fn expandable(text: &str) -> String {
    let bytes: Vec<String> = text
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("hex(2):{}", bytes.join(","))
}

fn file(sections: &[String]) -> String {
    let mut text = String::from("Windows Registry Editor Version 5.00\r\n");
    for section in sections {
        text.push_str("\r\n");
        text.push_str(&section.replace('\n', "\r\n"));
        text.push_str("\r\n");
    }
    text
}

/// The class `id` of an entry, under `classes` (64- or 32-bit).
fn class_sections(classes: &str, id: &str, name: &str, icon: &str, folder: &str) -> [String; 6] {
    [
        format!(
            "[{classes}\\{id}]\n@={name}\n\
             \"System.IsPinnedToNameSpaceTree\"=dword:00000001\n\
             \"SortOrderIndex\"=dword:00000042"
        ),
        format!("[{classes}\\{id}\\DefaultIcon]\n@={icon}"),
        format!(
            "[{classes}\\{id}\\InProcServer32]\n@={}",
            expandable(SHELL32)
        ),
        format!("[{classes}\\{id}\\Instance]\n\"CLSID\"=\"{FOLDER_SHORTCUT}\""),
        format!(
            "[{classes}\\{id}\\Instance\\InitPropertyBag]\n\
             \"Attributes\"=dword:00000011\n\"TargetFolderPath\"={folder}"
        ),
        format!(
            "[{classes}\\{id}\\ShellFolder]\n\
             \"FolderValueFlags\"=dword:00000028\n\"Attributes\"=dword:f080004d"
        ),
    ]
}

/// The `.reg` file that writes the entry, in full, the list's value first.
pub fn entry_file(entry: &Entry, list: &str) -> String {
    let id = class_id(entry.stick_id);
    let name = quoted(entry.name);
    let hex = crate::state::stick_hex(entry.stick_id);
    let icon = quoted(&format!("{},0", entry.icon.display()));
    let folder = quoted(&entry.folder.display().to_string());
    let mut sections = vec![format!("[{LISTS}\\{list}]\n\"{id}\"=\"{hex}\"")];
    for classes in [CLASSES, CLASSES_32] {
        sections.extend(class_sections(classes, &id, &name, &icon, &folder));
    }
    sections.push(format!("[{EXPLORER}\\Desktop\\NameSpace\\{id}]\n@={name}"));
    sections.push(format!(
        "[{EXPLORER}\\HideDesktopIcons\\NewStartPanel]\n\"{id}\"=dword:00000001"
    ));
    file(&sections)
}

/// The `.reg` file that removes the entry `class_id`, its list value last.
pub fn removal_file(class_id: &str, list: &str) -> String {
    file(&[
        format!("[-{CLASSES}\\{class_id}]"),
        format!("[-{CLASSES_32}\\{class_id}]"),
        format!("[-{EXPLORER}\\Desktop\\NameSpace\\{class_id}]"),
        format!("[{EXPLORER}\\HideDesktopIcons\\NewStartPanel]\n\"{class_id}\"=-"),
        list_value_removal(class_id, list),
    ])
}

/// The section that removes only the list's value `name`.
fn list_value_removal(name: &str, list: &str) -> String {
    format!("[{LISTS}\\{list}]\n{}=-", quoted(name))
}

/// The `.reg` file that removes list values that are not entries of ours.
pub fn odd_values_file(names: &[String], list: &str) -> String {
    file(
        &names
            .iter()
            .map(|name| list_value_removal(name, list))
            .collect::<Vec<_>>(),
    )
}

/// The string values in what `reg query` prints for a list:
/// `    <name>    REG_SZ    <data>` per value; other lines are skipped.
pub fn listed_in(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("    "))
        .filter_map(|line| line.split_once("    REG_SZ    "))
        .map(|(name, data)| (name.to_owned(), data.trim_end().to_owned()))
        .collect()
}

/// What the start removes from a list (ADR 0023 §7).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LeftBehind {
    /// Entries of ours whose stick is no longer an app stick here.
    pub entries: Vec<String>,
    /// List values that are not entries of ours: only the value goes.
    pub odd: Vec<String>,
}

/// Sorts the `listed` values: a value is ours if its name is the class id
/// derived from the stick id it holds; it is left behind if that stick is
/// not one of `app_sticks`.
pub fn left_behind(listed: &[(String, String)], app_sticks: &[StickId]) -> LeftBehind {
    let mut found = LeftBehind::default();
    for (name, data) in listed {
        match crate::state::stick_id(data) {
            Ok(id) if class_id(&id) == *name => {
                if !app_sticks.contains(&id) {
                    found.entries.push(name.clone());
                }
            }
            _ => found.odd.push(name.clone()),
        }
    }
    found
}

#[cfg(windows)]
pub mod platform {
    use super::*;
    use anyhow::{bail, Context, Result};
    use ourstick_core::OwnedStick;
    use std::path::PathBuf;

    fn list() -> Result<(PathBuf, String)> {
        let data_dir = ourstick_core::device::data_dir()?;
        let list = list_name(&data_dir);
        Ok((data_dir, list))
    }

    /// Imports `text` as a `.reg` file (UTF-16LE with a BOM, as `reg`
    /// reads it whatever the code page), named after `tag`.
    fn import(data_dir: &Path, tag: &str, text: &str) -> Result<()> {
        let path = data_dir.join(format!("pane-{tag}.reg"));
        let bytes: Vec<u8> = [0xFEFF]
            .into_iter()
            .chain(text.encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect();
        std::fs::write(&path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
        let output = crate::drive::no_window("reg")
            .arg("import")
            .arg(&path)
            .arg("/reg:64")
            .output();
        let _ = std::fs::remove_file(&path);
        let output = output.context("cannot run reg")?;
        if !output.status.success() {
            bail!(
                "reg import failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    /// Writes the stick's entry (again, in full): its name, its folder,
    /// the app's icon.
    pub fn show(stick_id: &StickId, name: &str, folder: &Path) -> Result<()> {
        let (data_dir, list) = list()?;
        let icon = std::env::current_exe().context("cannot find the app's own program")?;
        let entry = Entry {
            stick_id,
            name,
            folder,
            icon: &icon,
        };
        import(&data_dir, &class_id(stick_id), &entry_file(&entry, &list))
            .context("cannot show the stick in File Explorer's pane")
    }

    /// Removes the stick's entry, if there is one.
    pub fn hide(stick_id: &StickId) -> Result<()> {
        let (data_dir, list) = list()?;
        let id = class_id(stick_id);
        import(&data_dir, &id, &removal_file(&id, &list))
            .context("cannot take the stick out of File Explorer's pane")
    }

    /// [`show`] for a stick being mounted; a failure is logged, never
    /// stops the mount.
    pub fn show_logged(stick: &OwnedStick) {
        if let Err(error) = show(&stick.stick_id, &stick.name, &stick.root_path) {
            tracing::warn!("{error:#}");
        }
    }

    /// Removes this data folder's entries whose stick is not one of
    /// `app_sticks` (ADR 0023 §7). Best effort, logged.
    pub fn clean_up(app_sticks: &[StickId]) {
        let Ok((data_dir, list)) = list() else {
            return;
        };
        let key = format!("{LISTS}\\{list}");
        let Ok(output) = crate::drive::no_window("reg")
            .args(["query", &key, "/reg:64"])
            .output()
        else {
            return;
        };
        if !output.status.success() {
            // No list yet.
            return;
        }
        let found = left_behind(
            &listed_in(&String::from_utf8_lossy(&output.stdout)),
            app_sticks,
        );
        for id in &found.entries {
            match import(&data_dir, id, &removal_file(id, &list)) {
                Ok(()) => tracing::info!("removed a File Explorer entry left behind"),
                Err(error) => tracing::warn!("cannot remove an entry left behind: {error:#}"),
            }
        }
        if !found.odd.is_empty() {
            tracing::warn!(
                "{} value(s) in the File Explorer list are not entries of ours; only they go",
                found.odd.len()
            );
            if let Err(error) = import(&data_dir, "odd", &odd_values_file(&found.odd, &list)) {
                tracing::warn!("{error:#}");
            }
        }
    }
}

#[cfg(test)]
mod tests;
