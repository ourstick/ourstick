//! The paths a phone gives the core for what the user chose (ADR 0029):
//! `/ourstick-chosen/<the content URI, in hex>/<its name>`, and the names
//! inside a chosen folder after that. The URI travels in the path, so a
//! linked entry's stored path names its place with nothing kept beside it,
//! and the name is the last part, as on a desktop. Nothing on disk is ever
//! at these paths.

use std::path::{Component, Path, PathBuf};

use ourstick_core::places::{relation, Relation};

/// Where every chosen place's path begins.
const PREFIX: &str = "/ourstick-chosen";

/// The path of the place `uri`, shown as `name` (made usable first: a
/// provider may name a document anything).
pub fn place_path(uri: &str, name: &str) -> PathBuf {
    let mut hex = String::with_capacity(uri.len() * 2);
    for byte in uri.bytes() {
        hex.push_str(&format!("{byte:02x}"));
    }
    Path::new(PREFIX).join(hex).join(usable_name(name))
}

/// `name` with ` (n)` before its extension: `IMG_1 (2).jpg`, `Notes (3)`.
pub fn numbered(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => format!("{stem} ({n}).{extension}"),
        _ => format!("{name} ({n})"),
    }
}

/// The chosen place `place` under the name `name` instead.
pub fn renamed(place: &Path, name: &str) -> PathBuf {
    match place.parent() {
        Some(parent) => parent.join(usable_name(name)),
        None => place.to_path_buf(),
    }
}

/// `name` as a part of a path: `/` and `\` become `_`, control characters
/// go, and a name left empty, `.` or `..` is "Unnamed".
pub fn usable_name(name: &str) -> String {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    let name = name.trim();
    if is_usable_name(name) {
        name.to_owned()
    } else {
        "Unnamed".to_owned()
    }
}

/// What a chosen path names.
#[derive(Debug, PartialEq, Eq)]
pub struct Chosen {
    /// The place's content URI.
    pub uri: String,
    /// The path of the place itself.
    pub place: PathBuf,
    /// Whether the path is the place itself, not something inside it.
    pub is_place: bool,
}

/// Whether `path` is a chosen place's or inside one.
pub fn is_chosen(path: &Path) -> bool {
    path.starts_with(PREFIX)
}

/// The place `path` is, or lies inside; `None` if it is not a chosen path
/// or is malformed.
pub fn chosen(path: &Path) -> Option<Chosen> {
    let mut parts = path.strip_prefix(PREFIX).ok()?.components();
    let Some(Component::Normal(hex)) = parts.next() else {
        return None;
    };
    let Some(Component::Normal(name)) = parts.next() else {
        return None;
    };
    if parts
        .clone()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    let hex = hex.to_str()?;
    if hex.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    Some(Chosen {
        uri: String::from_utf8(bytes).ok()?,
        place: Path::new(PREFIX).join(hex).join(name),
        is_place: parts.next().is_none(),
    })
}

/// How the chosen paths `a` and `b` relate, when the paths alone tell:
/// both of one place (one URI, also chosen twice under two names), by the
/// names inside it. `None` when they are of two places, or not both
/// chosen: only Android can tell then.
pub fn relation_within(a: &Path, b: &Path) -> Option<Option<Relation>> {
    let (first, second) = (chosen(a)?, chosen(b)?);
    if first.uri != second.uri {
        return None;
    }
    let inside = |path: &Path, place: &Path| {
        Path::new("/").join(path.strip_prefix(place).unwrap_or(Path::new("")))
    };
    Some(relation(
        &inside(a, &first.place),
        &inside(b, &second.place),
    ))
}

/// Whether `name`, as a provider gives it, can be a part of a manifest
/// path: not empty, not `.` or `..`, no `/`, no `\` and no control
/// character (the manifest refuses those, and one would stop the whole
/// pass). Others are left out of a listing.
pub fn is_usable_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    const URI: &str =
        "content://com.android.externalstorage.documents/tree/primary%3ADCIM%2FCamera";

    #[test]
    fn a_place_and_what_is_inside_it_name_their_uri() {
        let place = place_path(URI, "Camera");
        assert!(is_chosen(&place));
        assert_eq!(place.file_name().unwrap(), "Camera");
        let found = chosen(&place).unwrap();
        assert_eq!(found.uri, URI);
        assert!(found.is_place);
        assert_eq!(found.place, place);

        let inside = place.join("2024").join("IMG_1.jpg");
        let found = chosen(&inside).unwrap();
        assert_eq!(found.uri, URI);
        assert!(!found.is_place);
        assert_eq!(found.place, place);
    }

    #[test]
    fn one_place_is_compared_by_the_names_inside_it() {
        let uri = "content://com.android.externalstorage.documents/tree/primary%3ADCIM";
        let dcim = place_path(uri, "DCIM");
        let again = place_path(uri, "DCIM (2)");
        assert_eq!(relation_within(&dcim, &again), Some(Some(Relation::Is)));
        assert_eq!(
            relation_within(&dcim, &again.join("Camera")),
            Some(Some(Relation::Contains))
        );
        assert_eq!(
            relation_within(&dcim.join("Camera/a.jpg"), &again.join("Camera")),
            Some(Some(Relation::IsInside))
        );
        assert_eq!(
            relation_within(&dcim.join("Camera"), &dcim.join("Screenshots")),
            Some(None)
        );
        // Two places, or a path that is not chosen: Android tells.
        let other = place_path("content://example/tree/2", "Camera");
        assert_eq!(relation_within(&dcim, &other), None);
        assert_eq!(relation_within(&dcim, Path::new("/home/alice")), None);
    }

    #[test]
    fn other_paths_are_not_chosen() {
        assert!(!is_chosen(Path::new("/storage/emulated/0/DCIM")));
        assert!(!is_chosen(Path::new("/ourstick-chosen-not")));
        assert_eq!(chosen(Path::new("/storage/emulated/0")), None);
        assert_eq!(chosen(Path::new("/ourstick-chosen")), None);
        assert_eq!(chosen(Path::new("/ourstick-chosen/6162")), None);
        assert_eq!(chosen(Path::new("/ourstick-chosen/616/x")), None);
        assert_eq!(chosen(Path::new("/ourstick-chosen/zz/x")), None);
        assert_eq!(chosen(Path::new("/ourstick-chosen/6162/x/../y")), None);
    }

    #[test]
    fn names_that_cannot_be_a_part_are_left_out() {
        for name in ["", ".", "..", "a/b", "nul\0", "a\\b", "line\nbreak"] {
            assert!(!is_usable_name(name), "{name:?}");
        }
        for name in ["IMG_1.jpg", ".nomedia", "a..b", "Ελληνικά"] {
            assert!(is_usable_name(name), "{name:?}");
        }
    }
}

#[cfg(test)]
mod naming_tests {
    use super::*;

    #[test]
    fn a_place_gets_a_usable_name_whatever_the_provider_said() {
        assert_eq!(usable_name("IMG_1.jpg"), "IMG_1.jpg");
        assert_eq!(usable_name("a/b\\c"), "a_b_c");
        assert_eq!(usable_name(" x\ny "), "xy");
        for bad in ["", ".", "..", "/", "\n"] {
            let fixed = usable_name(bad);
            assert!(is_usable_name(&fixed), "{bad:?} -> {fixed:?}");
        }
        let place = place_path("content://x/y", "/etc/passwd");
        let found = chosen(&place).unwrap();
        assert!(found.is_place);
        assert_eq!(place.file_name().unwrap(), "_etc_passwd");
    }
}

#[cfg(test)]
mod numbering_tests {
    use super::*;

    #[test]
    fn a_clashing_name_is_numbered_before_its_extension() {
        assert_eq!(numbered("IMG_0001.jpg", 2), "IMG_0001 (2).jpg");
        assert_eq!(numbered("archive.tar.gz", 3), "archive.tar (3).gz");
        assert_eq!(numbered("Notes", 2), "Notes (2)");
        assert_eq!(numbered(".nomedia", 2), ".nomedia (2)");
        let place = place_path("content://x/y", "IMG_0001.jpg");
        let other = renamed(&place, "IMG_0001 (2).jpg");
        assert_eq!(chosen(&other).unwrap().uri, "content://x/y");
        assert_eq!(other.file_name().unwrap(), "IMG_0001 (2).jpg");
    }
}
