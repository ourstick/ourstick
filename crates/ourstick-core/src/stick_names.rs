//! A stick's name on this device, and the folder named after it (ADR 0009
//! §3). A name is stored in one form (Unicode NFC, trimmed), so "Μαρία"
//! typed two ways is one name; the folder's name comes from it by the rules
//! a reader applies to paths, Windows' always, so a folder made anywhere can
//! go anywhere.

use icu_normalizer::ComposingNormalizerBorrowed;

use crate::download::files::component_problem;
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

/// Windows' reserved device names, with or without an extension.
fn is_reserved(name: &str) -> bool {
    component_problem(name, true) == Some("it is a reserved device name on Windows")
}

/// The folder for a stick called `name`: what Windows refuses replaced by
/// `_`, no trailing dot or space, a reserved name (`CON`, `COM1`…) given a
/// `_`, and " (2)", " (3)"… if `taken` says the name is used (the caller
/// compares ignoring case and asks the disk too). Always a plain name that
/// every system accepts.
pub fn folder_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut folder: String = normal_form(name)
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    folder = folder.trim_end_matches(['.', ' ']).to_owned();
    if folder.is_empty() || folder == "." || folder == ".." {
        folder = "Stick".to_owned();
    }
    if is_reserved(&folder) {
        // Windows reads the name before the first dot: `COM1.txt` is COM1.
        let stem_end = folder.find('.').unwrap_or(folder.len());
        folder.insert(stem_end, '_');
    }
    let folder = free_name_where(&folder, taken);
    debug_assert!(component_problem(&folder, true).is_none(), "{folder}");
    folder
}

#[cfg(test)]
mod tests;
