//! A phone's chosen places under a name no entry has (ADR 0029): what a
//! phone chose cannot be renamed there, so a clash is numbered instead of
//! refused.

use std::path::{Path, PathBuf};

use ourstick_core::linked::check_next_to_top;
use ourstick_core::sender_files::SenderFiles;
use ourstick_core::LinkedEntry;

use super::paths::{chosen, numbered, renamed};

/// `text`, a chosen place on Android, under the first of its name, then
/// `name (2)`, `name (3)`…, that no entry has, ignoring case; other paths
/// as they are.
pub fn free_name(
    files: &dyn SenderFiles,
    text: &str,
    taken: &[(String, PathBuf)],
    top: Option<&LinkedEntry>,
) -> String {
    let path = Path::new(text);
    let (Some(_), Some(name)) = (chosen(path), path.file_name()) else {
        return text.to_owned();
    };
    let name = name.to_string_lossy().into_owned();
    let is_free = |candidate: &str| {
        let lower = candidate.to_lowercase();
        !taken.iter().any(|(other, _)| other.to_lowercase() == lower)
            && top.is_none_or(|top| check_next_to_top(files, top, candidate).is_ok())
    };
    if is_free(&name) {
        return text.to_owned();
    }
    (2..1000)
        .map(|n| numbered(&name, n))
        .find(|candidate| is_free(candidate))
        .map_or_else(
            || text.to_owned(),
            |free| renamed(path, &free).display().to_string(),
        )
}
