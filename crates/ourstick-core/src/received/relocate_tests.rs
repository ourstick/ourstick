use super::*;
use crate::store::{DownloadedFile, JoinedStick, MemoryStore};

struct World {
    _dir: tempfile::TempDir,
    data: PathBuf,
    from: PathBuf,
    elsewhere: PathBuf,
    store: MemoryStore,
    id: StickId,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let from = dir.path().join("Downloads/Holiday");
    let elsewhere = dir.path().join("Expansion");
    std::fs::create_dir_all(from.join("Summer")).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(from.join("Summer/sea.jpg"), b"sea").unwrap();
    std::fs::write(from.join("notes.txt"), b"notes").unwrap();
    let store = MemoryStore::new();
    let id = crate::stick_id::StickSigningKey::from_bytes(&[7; 32]).stick_id();
    store
        .add_joined_stick(&JoinedStick {
            kind: crate::store::StickKind::App,
            ..JoinedStick::new(id, &from, 1)
        })
        .unwrap();
    store
        .record_downloaded_file(
            &id,
            &DownloadedFile {
                path: "notes.txt".to_owned(),
                saved_as: "notes.txt".to_owned(),
                stamp: crate::manifest::FileStamp {
                    size: 5,
                    mtime_ns: 1,
                    ctime_ns: 0,
                },
                blob_hash: [1; 32],
            },
        )
        .unwrap();
    World {
        data,
        from,
        elsewhere,
        store,
        id,
        _dir: dir,
    }
}

fn lock(world: &World) -> Lock {
    crate::locks::download_lock(&world.data, &world.id).unwrap()
}

#[test]
fn the_folder_moves_with_its_files_and_what_came_stays_recorded() {
    let world = world();
    let held = lock(&world);
    let to = move_received_folder(
        &world.store,
        &world.data,
        &world.id,
        &world.elsewhere,
        &held,
    )
    .unwrap();
    assert_eq!(to, world.elsewhere.join("Holiday"));
    assert!(!world.from.exists());
    assert_eq!(std::fs::read(to.join("Summer/sea.jpg")).unwrap(), b"sea");
    let joined = world.store.joined_stick(&world.id).unwrap().unwrap();
    assert_eq!(joined.output_dir, to);
    assert_eq!(world.store.downloaded_files(&world.id).unwrap().len(), 1);
}

#[test]
fn across_disks_it_is_copied_whole_with_its_times_then_the_old_one_goes() {
    let world = world();
    let old = std::fs::metadata(world.from.join("notes.txt"))
        .unwrap()
        .modified()
        .unwrap();
    let to = world.elsewhere.join("Holiday");
    let other_disk =
        |_: &Path, _: &Path| Err(std::io::Error::from(std::io::ErrorKind::CrossesDevices));
    move_folder(&world.from, &to, &other_disk).unwrap();
    assert!(!world.from.exists());
    assert_eq!(std::fs::read(to.join("notes.txt")).unwrap(), b"notes");
    let new = std::fs::metadata(to.join("notes.txt"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(new, old, "the time stays: what came is still known as such");
}

#[test]
fn a_copy_that_fails_leaves_the_old_folder_whole_and_no_half_copy() {
    let world = world();
    let to = world.elsewhere.join("Holiday");
    // The place it would go is taken by a file: the copy cannot start.
    std::fs::write(world.elsewhere.join("Holiday"), b"x").unwrap();
    let other_disk =
        |_: &Path, _: &Path| Err(std::io::Error::from(std::io::ErrorKind::CrossesDevices));
    assert!(move_folder(&world.from, &to, &other_disk).is_err());
    assert_eq!(
        std::fs::read(world.from.join("Summer/sea.jpg")).unwrap(),
        b"sea"
    );
    assert_eq!(
        std::fs::read(world.elsewhere.join("Holiday")).unwrap(),
        b"x"
    );
}

#[test]
fn a_place_taken_or_not_allowed_is_refused_and_nothing_moves() {
    let world = world();
    let held = lock(&world);
    std::fs::create_dir(world.elsewhere.join("Holiday")).unwrap();
    let taken = move_received_folder(
        &world.store,
        &world.data,
        &world.id,
        &world.elsewhere,
        &held,
    );
    assert!(
        matches!(taken, Err(MoveError::AlreadyThere { .. })),
        "{taken:?}"
    );
    // Into itself, or into OurStick's data folder.
    for into in [world.from.join("Summer"), world.data.clone()] {
        let refused = move_received_folder(&world.store, &world.data, &world.id, &into, &held);
        assert!(refused.is_err(), "{}", into.display());
    }
    assert!(world.from.join("notes.txt").exists());
    // Where it is already: nothing to do.
    let parent = world.from.parent().unwrap().to_path_buf();
    assert_eq!(
        move_received_folder(&world.store, &world.data, &world.id, &parent, &held).unwrap(),
        world.from
    );
}
