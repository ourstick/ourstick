//! Names this computer can save (ADR 0010): a name the sender's system
//! allows and this one refuses gets the smallest change that makes it
//! savable, chosen here, with no question asked. One rule for every name
//! OurStick writes: the files of a received stick and the folder of a stick
//! (`stick_names::folder_name`).

use std::borrow::Cow;

use super::files::{component_problem, is_reserved_device, MAX_NAME};

/// What Windows does not allow in a name, beside control characters.
const NOT_ON_WINDOWS: [char; 7] = ['<', '>', ':', '"', '|', '?', '*'];

/// The room a name keeps for " (n)" when it is cut, so a numbered name
/// still fits (" (99999)").
const NUMBER_ROOM: usize = 8;

/// `name` (one file or folder name, never empty, as a manifest holds it) as
/// this computer can save it: unchanged if it can, else with the smallest
/// change (ADR 0010, rule 2). `windows` selects Windows' rules, a parameter
/// so they are tested everywhere. Idempotent; the result always passes
/// [`component_problem`].
pub(crate) fn safe_name(name: &str, windows: bool) -> Cow<'_, str> {
    if component_problem(name, windows).is_none() {
        return Cow::Borrowed(name);
    }
    let mut changed: String = if windows {
        name.chars()
            .map(|c| {
                if c.is_control() || NOT_ON_WINDOWS.contains(&c) {
                    '_'
                } else {
                    c
                }
            })
            .collect()
    } else {
        name.to_owned()
    };
    // `c:` would be a drive on Windows: refused everywhere (a stick must
    // not hold one on any receiver).
    let bytes = changed.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        changed.replace_range(1..2, "_");
    }
    if length(&changed, windows) > MAX_NAME {
        changed = cut(&changed, MAX_NAME - NUMBER_ROOM, windows);
    }
    if windows {
        windows_ends(&mut changed);
    }
    if component_problem(&changed, windows).is_some() {
        // No rule above made it savable (it should not happen): the plainest
        // name there is.
        tracing::warn!("a name no rule could make savable; saved as \"_\"");
        changed = "_".to_owned();
    }
    Cow::Owned(changed)
}

/// Windows' rules for a name's end and stem: no trailing dot or space (`_`
/// if nothing is left), and a name Windows keeps for itself gets `_` after
/// its stem (`CON.txt` → `CON_.txt`).
fn windows_ends(name: &mut String) {
    let kept = name.trim_end_matches(['.', ' ']).len();
    name.truncate(kept);
    if name.is_empty() {
        name.push('_');
    }
    if is_reserved_device(name) {
        // Windows reads the name before the first dot: `COM1.txt` is COM1.
        let stem_end = name.find('.').unwrap_or(name.len());
        name.insert(stem_end, '_');
    }
}

/// `name` numbered for a second, third… file at one place: "photo (2).jpg",
/// the folder "Trip (2)"; its stem cut if the number would make it too long.
pub(crate) fn numbered(name: &str, number: u32, windows: bool) -> String {
    let (stem, extension) = split_extension(name);
    let suffix = format!(" ({number}){extension}");
    let room = MAX_NAME.saturating_sub(length(&suffix, windows));
    let stem = if length(stem, windows) > room {
        cut_to(stem, room, windows)
    } else {
        stem.to_owned()
    };
    let numbered = format!("{stem}{suffix}");
    safe_name(&numbered, windows).into_owned()
}

/// A name's stem and extension (with its dot): "photo.jpg" → ("photo",
/// ".jpg"). A name starting with its only dot (".profile") has none.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    }
}

/// How long the system counts `name`: UTF-16 units on Windows, bytes
/// elsewhere.
fn length(name: &str, windows: bool) -> usize {
    if windows {
        name.encode_utf16().count()
    } else {
        name.len()
    }
}

/// `name` cut to at most `max`, keeping its extension unless the extension
/// alone takes half the room.
fn cut(name: &str, max: usize, windows: bool) -> String {
    let (stem, extension) = split_extension(name);
    let extension_length = length(extension, windows);
    if extension_length * 2 > max {
        return cut_to(name, max, windows);
    }
    format!(
        "{}{extension}",
        cut_to(stem, max - extension_length, windows)
    )
}

/// The longest start of `text` of at most `max`, on a character boundary,
/// not ending inside an emoji's joined sequence.
fn cut_to(text: &str, max: usize, windows: bool) -> String {
    let mut kept = String::new();
    for c in text.chars() {
        let mut next = kept.clone();
        next.push(c);
        if length(&next, windows) > max {
            break;
        }
        kept = next;
    }
    let whole = kept.trim_end_matches(['\u{200d}', '\u{fe0f}']).len();
    kept.truncate(whole);
    kept
}

/// The first name of a path, if OurStick uses it itself in a received
/// folder, given a `_` in front (`.ourstick-partial` → `_.ourstick-partial`).
pub(crate) fn not_ours(first: &str, ours: &[&str]) -> Option<String> {
    ours.iter()
        .any(|name| first.eq_ignore_ascii_case(name))
        .then(|| format!("_{first}"))
}

#[cfg(test)]
mod tests;
