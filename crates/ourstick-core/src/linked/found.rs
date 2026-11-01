//! "Find it…" (ADR 0029): a linked entry that is not found where it was is
//! chosen again where it is now. It keeps its name, so the link and what
//! those who have it see stay the same, and its recorded files, so what did
//! not change is not hashed again.

use std::path::Path;

use super::{check_next_to_top, check_place, LinkError};
use crate::sender_files::SenderFiles;
use crate::stick_id::StickId;
use crate::store::{LinkedEntry, Store, StoreError};

/// "“Summer” was a folder. Choose that folder." Messages are for users.
pub(super) fn not_the_same_kind(name: &str, was_folder: bool) -> String {
    let what = if was_folder { "folder" } else { "file" };
    format!("\u{201c}{name}\u{201d} was a {what}. Choose that {what}.")
}

/// The linked entry `name` of the stick `stick_id`, at `path` instead: by
/// the same rules as a new link beside the stick's other entries, and of
/// the same kind, a folder or a file. The folder at the stick's top must
/// not hold an entry's name. Blocking.
pub fn plan_found(
    files: &dyn SenderFiles,
    store: &dyn Store,
    stick_id: &StickId,
    data_dir: &Path,
    name: &str,
    path: &Path,
) -> Result<LinkedEntry, LinkError> {
    let stick = store
        .owned_stick(stick_id)?
        .ok_or(StoreError::StickNotFound)?;
    let (entry, others): (Vec<LinkedEntry>, Vec<LinkedEntry>) = store
        .linked_entries(stick_id)?
        .into_iter()
        .partition(|entry| entry.name == name);
    let entry = entry
        .into_iter()
        .next()
        .ok_or_else(|| StoreError::LinkNotFound(name.to_owned()))?;
    let facts = check_place(
        files,
        store,
        stick_id,
        data_dir,
        &stick.root_path,
        &others,
        path,
    )?;
    if facts.is_folder() != entry.is_dir {
        return Err(LinkError::NotTheSameKind {
            name: entry.name,
            was_folder: entry.is_dir,
        });
    }
    let found = LinkedEntry {
        path: path.to_path_buf(),
        disk: facts.disk,
        ..entry
    };
    if found.at_top {
        for other in &others {
            check_next_to_top(files, &found, &other.name)?;
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::CipherId;
    use crate::sender_files::LocalFiles;
    use crate::stick_id::StickSigningKey;
    use crate::store::{MemoryStore, OwnedStick, StickKind};

    fn entry(name: &str, path: &Path, is_dir: bool, at_top: bool) -> LinkedEntry {
        LinkedEntry {
            name: name.to_owned(),
            path: path.to_path_buf(),
            is_dir,
            disk: None,
            added_at_ms: 1,
            at_top,
        }
    }

    /// A stick with no folder of its own: "Summer" at its top, gone, and
    /// "notes.txt" beside it.
    fn stick() -> (tempfile::TempDir, MemoryStore, StickId) {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new();
        let stick_id = StickSigningKey::from_bytes(&[1; 32]).stick_id();
        store
            .add_owned_stick(&OwnedStick {
                kind: StickKind::App,
                has_own_folder: false,
                ..OwnedStick::new(
                    stick_id,
                    "Summer",
                    dir.path().join("own"),
                    CipherId::XChaCha20Poly1305,
                    1,
                )
            })
            .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "n").unwrap();
        for added in [
            entry("Summer", &dir.path().join("Summer"), true, true),
            entry("notes.txt", &dir.path().join("notes.txt"), false, false),
        ] {
            store.add_linked_entry(&stick_id, &added).unwrap();
        }
        (dir, store, stick_id)
    }

    fn find(
        dir: &tempfile::TempDir,
        store: &MemoryStore,
        stick_id: &StickId,
        name: &str,
        path: &Path,
    ) -> Result<LinkedEntry, LinkError> {
        plan_found(
            &LocalFiles,
            store,
            stick_id,
            &dir.path().join("data"),
            name,
            path,
        )
    }

    #[test]
    fn a_folder_found_elsewhere_keeps_its_name_and_place_at_the_top() {
        let (dir, store, stick_id) = stick();
        let moved = dir.path().join("disk/Summer 2026");
        std::fs::create_dir_all(&moved).unwrap();
        let found = find(&dir, &store, &stick_id, "Summer", &moved).unwrap();
        assert_eq!(found.name, "Summer");
        assert_eq!(found.path, moved);
        assert!(found.at_top && found.is_dir);
    }

    #[test]
    fn what_is_found_is_of_the_same_kind_and_touches_no_other_entry() {
        let (dir, store, stick_id) = stick();
        let refused = find(
            &dir,
            &store,
            &stick_id,
            "Summer",
            &dir.path().join("notes.txt"),
        );
        assert!(
            matches!(refused, Err(LinkError::Overlaps { .. })),
            "{refused:?}"
        );
        let file = dir.path().join("photo.jpg");
        std::fs::write(&file, "p").unwrap();
        let refused = find(&dir, &store, &stick_id, "Summer", &file).unwrap_err();
        assert_eq!(
            refused.to_string(),
            "“Summer” was a folder. Choose that folder."
        );
        assert!(matches!(
            find(&dir, &store, &stick_id, "Winter", &file),
            Err(LinkError::Store(StoreError::LinkNotFound(_)))
        ));
    }

    #[test]
    fn a_folder_at_the_top_may_not_hold_another_entrys_name() {
        let (dir, store, stick_id) = stick();
        let moved = dir.path().join("disk/Summer");
        std::fs::create_dir_all(&moved).unwrap();
        std::fs::write(moved.join("notes.txt"), "other").unwrap();
        assert!(matches!(
            find(&dir, &store, &stick_id, "Summer", &moved),
            Err(LinkError::InTop { .. })
        ));
    }
}
