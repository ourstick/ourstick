use super::*;

fn entry(name: &str, path: &Path, is_dir: bool) -> LinkedEntry {
    let metadata = std::fs::metadata(path).ok();
    LinkedEntry {
        name: name.to_owned(),
        path: path.to_path_buf(),
        is_dir,
        disk: metadata.as_ref().and_then(disk_of),
        added_at_ms: 1,
    }
}

#[test]
fn a_path_finds_its_root_by_its_first_name() {
    let layout = StickLayout::new(
        Path::new("/stick"),
        &[
            entry("Films", Path::new("/media/disk/Films"), true),
            entry("notes.txt", Path::new("/home/me/notes.txt"), false),
        ],
    );
    assert_eq!(
        layout.on_disk("Films/2024/a.mkv"),
        Path::new("/media/disk/Films/2024/a.mkv")
    );
    assert_eq!(layout.on_disk("notes.txt"), Path::new("/home/me/notes.txt"));
    assert_eq!(layout.on_disk("own/b.txt"), Path::new("/stick/own/b.txt"));
    // Exactly: another case is the own folder's (it is shadowed there).
    assert_eq!(layout.on_disk("films/x"), Path::new("/stick/films/x"));
    assert_eq!(layout.root_of("Films/a").name.as_deref(), Some("Films"));
    assert_eq!(layout.root_of("Filmsx").name, None);
    assert!(layout.is_shadowed("FILMS"));
    assert!(!layout.is_shadowed("Films 2"));
}

#[test]
fn a_root_is_gone_when_missing_or_no_longer_what_was_linked() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Films");
    let file = dir.path().join("notes.txt");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(&file, "x").unwrap();
    let layout = StickLayout::new(
        dir.path(),
        &[
            entry("Films", &folder, true),
            entry("notes.txt", &file, false),
        ],
    );
    assert!(layout.linked().iter().all(|root| !root.is_gone()));
    assert!(!layout.own().is_gone());

    // A file where the folder was, and the reverse.
    std::fs::remove_dir(&folder).unwrap();
    std::fs::write(&folder, "now a file").unwrap();
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert!(layout.linked().iter().all(Root::is_gone));
    // Missing.
    std::fs::remove_file(&folder).unwrap();
    assert!(layout.linked()[0].is_gone());
    // On another disk than when linked.
    std::fs::create_dir(&folder).unwrap();
    let moved = Root {
        disk: layout.linked()[0].disk.map(|disk| disk.wrapping_add(1)),
        ..layout.linked()[0].clone()
    };
    #[cfg(unix)]
    assert!(moved.is_gone());
    #[cfg(not(unix))]
    assert!(!moved.is_gone());
}

#[test]
fn the_layout_is_loaded_from_the_store() {
    use crate::cipher::CipherId;
    use crate::stick_id::StickSigningKey;
    use crate::store::{MemoryStore, OwnedStick};
    let store = MemoryStore::new();
    let stick_id = StickSigningKey::from_bytes(&[1; 32]).stick_id();
    store
        .add_owned_stick(&OwnedStick::new(
            stick_id,
            "s",
            "/stick",
            CipherId::XChaCha20Poly1305,
            1,
        ))
        .unwrap();
    let films = entry("Films", Path::new("/media/Films"), true);
    store.add_linked_entry(&stick_id, &films).unwrap();
    let layout = StickLayout::load(&store, &stick_id).unwrap();
    assert_eq!(layout, StickLayout::new(Path::new("/stick"), &[films]));
    let other = StickSigningKey::from_bytes(&[2; 32]).stick_id();
    assert!(matches!(
        StickLayout::load(&store, &other),
        Err(StoreError::StickNotFound)
    ));
}
