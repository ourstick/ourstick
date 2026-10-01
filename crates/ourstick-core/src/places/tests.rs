use super::*;
use crate::cipher::CipherId;
use crate::stick_id::StickSigningKey;
use crate::store::{LinkedEntry, MemoryStore, OwnedStick};

fn add(store: &MemoryStore, seed: u8, name: &str, root: &Path, kind: StickKind) -> StickId {
    let stick_id = StickSigningKey::from_bytes(&[seed; 32]).stick_id();
    store
        .add_owned_stick(&OwnedStick {
            kind,
            ..OwnedStick::new(stick_id, name, root, CipherId::XChaCha20Poly1305, 1)
        })
        .unwrap();
    stick_id
}

/// The shapes of a real device: command-line sticks of a disk and of a
/// folder in it, the app's stick linking that same folder, stale roots.
struct Device {
    _dir: tempfile::TempDir,
    store: MemoryStore,
    data: PathBuf,
    app: PathBuf,
    disk: PathBuf,
    documentaries: PathBuf,
}

fn device() -> Device {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let app = dir.path().join("home/OurStick");
    let disk = dir.path().join("G/Rescued_Disk");
    let documentaries = disk.join("0videos");
    for folder in [&data, &app, &documentaries] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let store = MemoryStore::new();
    add(&store, 1, "Rescued_Disk", &disk, StickKind::Cli);
    add(&store, 2, "0videos", &documentaries, StickKind::Cli);
    add(
        &store,
        3,
        "gone",
        &dir.path().join("tmp/share"),
        StickKind::Cli,
    );
    let app_id = add(&store, 4, "OurStick", &app, StickKind::App);
    store
        .add_linked_entry(
            &app_id,
            &LinkedEntry {
                name: "0videos".to_owned(),
                path: documentaries.clone(),
                is_dir: true,
                disk: None,
                added_at_ms: 1,
            },
        )
        .unwrap();
    Device {
        store,
        data,
        app,
        disk,
        documentaries,
        _dir: dir,
    }
}

fn check(device: &Device, path: &Path) -> Result<(), PlaceError> {
    check_new_stick_folder(&device.store, &device.data, path)
}

#[test]
fn what_a_device_already_has_may_still_be_shared_again() {
    let device = device();
    // Folders already in command-line sticks, and in an app stick's link.
    check(&device, &device.disk).unwrap();
    check(&device, &device.documentaries).unwrap();
    check(&device, &device.documentaries.join("one film")).unwrap();
}

#[test]
fn an_app_sticks_folder_and_the_data_folder_are_ourstick_s_alone() {
    let device = device();
    for (path, relation) in [
        (device.app.clone(), Relation::Is),
        (device.app.join("photos"), Relation::IsInside),
        (
            device.app.parent().unwrap().to_path_buf(),
            Relation::Contains,
        ),
        (device.data.join("cache"), Relation::IsInside),
    ] {
        match check(&device, &path) {
            Err(PlaceError::Taken {
                relation: found, ..
            }) => assert_eq!(found, relation),
            other => panic!("{}: {other:?}", path.display()),
        }
    }
}

#[test]
fn a_stick_being_removed_keeps_its_folder_until_it_is_gone() {
    let device = device();
    let app_id = StickSigningKey::from_bytes(&[4; 32]).stick_id();
    device.store.mark_removing(&app_id).unwrap();
    assert!(check(&device, &device.app).is_err());
    let taken = app_places(&device.store, &device.data, Some(&app_id)).unwrap();
    assert_eq!(taken.len(), 1, "the data folder only");
}

#[test]
fn places_compare_the_way_the_system_does() {
    // Windows, tried everywhere: a gone path and a resolved one compare
    // alike, network shares too, and the drive letter's case does not count.
    for (resolved, stored) in [
        (r"\\?\C:\Users\me\OurStick", r"c:\users\me\ourstick"),
        (r"\\?\UNC\server\share\Photos", r"\\SERVER\share\photos"),
    ] {
        assert_eq!(
            comparable_text(resolved, true),
            comparable_text(stored, true)
        );
    }
    assert_ne!(
        comparable_text(r"\\?\UNC\server\share", true),
        comparable_text(r"UNC\server\share", true)
    );
    let lower = comparable(Path::new("/nowhere/Photos"));
    let upper = comparable(Path::new("/nowhere/PHOTOS"));
    assert_eq!(lower == upper, cfg!(any(target_os = "macos", windows)));
    assert_eq!(
        relation(Path::new("/a"), Path::new("/a/b")),
        Some(Relation::Contains)
    );
    assert_eq!(relation(Path::new("/a/b"), Path::new("/a/bc")), None);
}

#[test]
fn a_macos_sticks_image_is_ourstick_s_too() {
    let device = device();
    let app_id = StickSigningKey::from_bytes(&[4; 32]).stick_id();
    let image = device
        .data
        .parent()
        .unwrap()
        .join("Images/OurStick.sparseimage");
    device.store.set_image_path(&app_id, &image).unwrap();
    assert!(check(&device, &image).is_err());
    assert!(
        check(&device, image.parent().unwrap()).is_err(),
        "it holds the image"
    );
}
