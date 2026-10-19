//! Which places on this device may become a stick's (ADR 0009 §5, as
//! reviewed): the one place that decides. An app stick's own folder is the
//! app's alone, since removing the stick deletes it, and so is OurStick's
//! data folder; nothing else may be, hold or lie inside them. Anything else
//! may be in several sticks: the same photos in a stick for each person.
//! Checked only when a stick or a link is made, never when one is shared, so
//! what was made before stays as it is. Blocking I/O.

use std::path::{Path, PathBuf};

use crate::stick_id::StickId;
use crate::store::{StickKind, Store, StoreError};

mod measure;
mod sending;

pub use measure::{measure, Measured};
pub use sending::{may_send, Ask, MaySend, SendRules};

/// How two places relate, for a message: `a` is, contains or lies inside `b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Is,
    Contains,
    IsInside,
}

impl Relation {
    pub fn words(self) -> &'static str {
        match self {
            Self::Is => "is",
            Self::Contains => "contains",
            Self::IsInside => "is inside",
        }
    }
}

/// `path` as places are compared: links resolved in the part of it that
/// exists (a folder not made yet, inside a linked folder, is compared where
/// it will really be), then [`comparable_text`] with letter case ignored
/// where the file system ignores it (macOS, Windows).
pub fn comparable(path: &Path) -> PathBuf {
    let resolved = resolved(path);
    PathBuf::from(comparable_text(
        &resolved.to_string_lossy(),
        cfg!(any(target_os = "macos", windows)),
    ))
}

/// `path` with links resolved in its longest part that exists. Relative
/// paths are taken from the current folder; `..` in a part that does not
/// exist yet is taken as written.
fn resolved(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if let Ok(real) = std::fs::canonicalize(&absolute) {
        return real;
    }
    let path = &lexical(&absolute);
    let mut rest = Vec::new();
    let mut existing = path.as_path();
    loop {
        if let Ok(real) = std::fs::canonicalize(existing) {
            return rest.iter().rev().fold(real, |path, part| path.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                existing = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// `path` with `.` and `..` folded away as written.
fn lexical(path: &Path) -> PathBuf {
    let mut folded = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                folded.pop();
            }
            other => folded.push(other),
        }
    }
    folded
}

/// The text of a path as places are compared: Windows' `\\?\` dropped and
/// `\\?\UNC\` made `\\` again (so a gone path compares like a resolved
/// one), and with `fold_case` all of it in lower case, the drive letter too.
pub fn comparable_text(text: &str, fold_case: bool) -> String {
    let text = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(text).to_owned()
    };
    if fold_case {
        text.to_lowercase()
    } else {
        text
    }
}

/// How `a` relates to `b`, both [`comparable`].
pub fn relation(a: &Path, b: &Path) -> Option<Relation> {
    if a == b {
        Some(Relation::Is)
    } else if b.starts_with(a) {
        Some(Relation::Contains)
    } else if a.starts_with(b) {
        Some(Relation::IsInside)
    } else {
        None
    }
}

/// A place another stick or OurStick itself has, and what to call it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    pub place: PathBuf,
    pub what: String,
}

/// The places no new stick folder or link may touch: OurStick's data folder
/// (which holds the cache) and every app stick's own folder, those being
/// removed too, but `except`'s (a stick's own folder is checked by the
/// stick's own rules).
pub fn app_places(
    store: &dyn Store,
    data_dir: &Path,
    except: Option<&StickId>,
) -> Result<Vec<Taken>, StoreError> {
    let mut taken = vec![Taken {
        place: comparable(data_dir),
        what: "OurStick's data folder".to_owned(),
    }];
    let sticks = store
        .owned_sticks()?
        .into_iter()
        .chain(store.removing_sticks()?);
    for stick in sticks {
        if stick.kind != StickKind::App || Some(&stick.stick_id) == except {
            continue;
        }
        let what = format!("the stick \u{201c}{}\u{201d}", stick.name);
        taken.push(Taken {
            place: comparable(&stick.root_path),
            what: what.clone(),
        });
        // macOS: the disk image it lives in.
        if let Some(image) = &stick.image_path {
            taken.push(Taken {
                place: comparable(image),
                what,
            });
        }
    }
    Ok(taken)
}

/// The first of `taken` that `path` is, contains or lies inside.
pub fn clash<'a>(path: &Path, taken: &'a [Taken]) -> Option<(Relation, &'a Taken)> {
    let path = comparable(path);
    taken
        .iter()
        .find_map(|taken| relation(&path, &taken.place).map(|relation| (relation, taken)))
}

/// Why a place cannot be a new stick's folder. Messages are for users.
#[derive(Debug, thiserror::Error)]
pub enum PlaceError {
    #[error("{} {} {}, which only OurStick may use", .path.display(), .relation.words(), .what)]
    Taken {
        path: PathBuf,
        relation: Relation,
        what: String,
    },
    #[error("{} {} {}: {}", .path.display(), .relation.words(), .what, .why)]
    Shared {
        path: PathBuf,
        relation: Relation,
        what: String,
        why: &'static str,
    },
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Whether `path` may be the folder of a new stick (a command-line one, or
/// a new app stick's) or where a download goes: not, nor around, nor
/// inside an app stick's folder or image, or the data folder. (Downloads
/// inside an app stick would be deleted with it.)
pub fn check_new_stick_folder(
    store: &dyn Store,
    data_dir: &Path,
    path: &Path,
) -> Result<(), PlaceError> {
    let taken = app_places(store, data_dir, None)?;
    match clash(path, &taken) {
        Some((relation, taken)) => Err(PlaceError::Taken {
            path: path.to_path_buf(),
            relation,
            what: taken.what.clone(),
        }),
        None => Ok(()),
    }
}

/// Whether `path` may be the folder of the received stick `stick_id`: as
/// for any new stick ([`check_new_stick_folder`]), and also not, nor around,
/// nor inside another received stick's folder (their unfinished files would
/// meet), a folder this device shares (a stick of the command line) or a
/// folder linked to one of its sticks (what it receives would be shared
/// onward).
pub fn check_received_folder(
    store: &dyn Store,
    data_dir: &Path,
    path: &Path,
    stick_id: &StickId,
) -> Result<(), PlaceError> {
    check_new_stick_folder(store, data_dir, path)?;
    let mut shared = Vec::new();
    for stick in store
        .owned_sticks()?
        .into_iter()
        .chain(store.removing_sticks()?)
    {
        if stick.kind != StickKind::App {
            shared.push(Taken {
                place: comparable(&stick.root_path),
                what: format!("the folder of your stick \u{201c}{}\u{201d}", stick.name),
            });
        }
        for linked in store.linked_entries(&stick.stick_id)? {
            shared.push(Taken {
                place: comparable(&linked.path),
                what: format!(
                    "{}, shared on \u{201c}{}\u{201d}",
                    linked.path.display(),
                    stick.name
                ),
            });
        }
    }
    if let Some((relation, taken)) = clash(path, &shared) {
        return Err(PlaceError::Shared {
            path: path.to_path_buf(),
            relation,
            what: taken.what.clone(),
            why: "what you receive there would be shared onward; choose another folder",
        });
    }
    let others: Vec<Taken> = store
        .joined_sticks()?
        .into_iter()
        .chain(store.forgetting_joined_sticks()?)
        .filter(|joined| joined.stick_id != *stick_id)
        .map(|joined| Taken {
            place: comparable(&joined.output_dir),
            what: format!(
                "the folder of the received stick \u{201c}{}\u{201d}",
                crate::received::display_name(joined.name.as_deref(), &joined.stick_id)
            ),
        })
        .collect();
    if let Some((relation, taken)) = clash(path, &others) {
        return Err(PlaceError::Shared {
            path: path.to_path_buf(),
            relation,
            what: taken.what.clone(),
            why: "each received stick needs a folder of its own; choose another folder",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
