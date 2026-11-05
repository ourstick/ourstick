//! A stick's name on this device, and the folder named after it (ADR 0009
//! §3). A name is stored in one form (Unicode NFC, trimmed), so "Μαρία"
//! typed two ways is one name; the folder's name comes from it by the rules
//! a reader applies to paths, Windows' always, so a folder made anywhere can
//! go anywhere.

use icu_normalizer::ComposingNormalizerBorrowed;

use crate::download::saved_names::safe_name;
use crate::drive::free_name_where;
use crate::manifest::{name_for_manifest, validate_name};
use crate::store::OwnedStick;

/// Why a name cannot be a stick's. The messages are meant for users.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("a stick needs a name")]
    Empty,
    #[error("a stick's name has at most 60 characters")]
    TooLong,
    #[error("a stick's name cannot hold invisible or control characters")]
    BadCharacter,
    #[error("another stick is called \u{201c}{0}\u{201d}")]
    Taken(String),
}

/// `text` in the one stored form: NFC, without spaces at either end.
pub fn normal_form(text: &str) -> String {
    ComposingNormalizerBorrowed::new_nfc()
        .normalize(text)
        .trim()
        .to_owned()
}

/// Whether two names are the same name: equal once in the stored form,
/// ignoring letter case the Unicode way.
pub fn same_name(a: &str, b: &str) -> bool {
    normal_form(a).to_lowercase() == normal_form(b).to_lowercase()
}

/// The stored form of a name the user gives a stick, if it may be one: 1 to
/// 60 characters, none invisible, not another live stick's name among
/// `others` (the app's sticks, the one renamed left out). Checked only when
/// a stick is made or renamed: names kept from before stay as they are.
pub fn new_stick_name(input: &str, others: &[OwnedStick]) -> Result<String, NameError> {
    let name = normal_form(input);
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.chars().count() > 60 {
        return Err(NameError::TooLong);
    }
    if validate_name(&name).is_err() {
        return Err(NameError::BadCharacter);
    }
    if let Some(other) = others.iter().find(|other| same_name(&other.name, &name)) {
        return Err(NameError::Taken(other.name.clone()));
    }
    Ok(name)
}

/// A name for a stick made from the command line, from `folder` (a folder's
/// name): never refused, made fit instead (invisible characters dropped,
/// cut to 60 characters), "stick" if nothing is left.
pub fn name_from_folder(folder: &str) -> String {
    let name = name_for_manifest(&normal_form(folder));
    if name.is_empty() {
        "stick".to_owned()
    } else {
        name
    }
}

/// The folder for a stick called `name`, by the rule every name OurStick
/// writes follows (`download::saved_names::safe_name`, Windows' rules always,
/// so a folder made anywhere can go anywhere): what Windows refuses replaced
/// by `_`, no trailing dot or space, a reserved name (`CON`, `COM1`…) given
/// a `_`, a name too long cut, a leading `-` made `_` (the folder is given
/// to programs such as `diskutil`, which would take it for an option);
/// "Stick" if nothing is left; and " (2)",
/// " (3)"… if `taken` says the name is used (the caller compares ignoring
/// case and asks the disk too).
pub fn folder_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut folder: String = normal_form(name)
        .chars()
        .map(|c| if matches!(c, '/' | '\\') { '_' } else { c })
        .collect();
    if folder.starts_with('-') {
        folder.replace_range(..1, "_");
    }
    let plain = folder.trim_end_matches(['.', ' ']);
    let folder = if plain.is_empty() {
        "Stick".to_owned()
    } else {
        safe_name(&folder, true).into_owned()
    };
    free_name_where(&folder, taken)
}

/// The folder a received stick goes into: [`folder_name`], and never one a
/// system opens as something other than a folder, since the sender chose
/// the name: a macOS bundle (`Invoice.app`, `.pkg`, `.saver`…), which
/// "Open folder" would hand to the system to run or install, or a Windows
/// shell folder (`Name.{CLSID}`). Such a name gets a `_` at its end.
pub fn received_folder_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let plain = folder_name(name, |_| false);
    if opens_as_something_else(&plain) {
        folder_name(&format!("{plain}_"), taken)
    } else {
        folder_name(name, taken)
    }
}

/// The ends macOS opens a folder by as a bundle, and Windows' shell folders.
fn opens_as_something_else(folder: &str) -> bool {
    const BUNDLES: [&str; 23] = [
        "app",
        "appex",
        "bundle",
        "component",
        "dext",
        "driver",
        "framework",
        "kext",
        "mdimporter",
        "mpkg",
        "osax",
        "pkg",
        "plugin",
        "prefpane",
        "qlgenerator",
        "saver",
        "scptd",
        "service",
        "systemextension",
        "wdgt",
        "workflow",
        "action",
        "xpc",
    ];
    let Some((_, end)) = folder.rsplit_once('.') else {
        return false;
    };
    let end = end.to_lowercase();
    BUNDLES.contains(&end.as_str()) || (end.starts_with('{') && end.ends_with('}'))
}

#[cfg(test)]
mod tests;
