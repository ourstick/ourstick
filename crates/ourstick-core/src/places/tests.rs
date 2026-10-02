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

fn received_into(device: &Device, seed: u8, output: &Path) -> StickId {
    let stick_id = StickSigningKey::from_bytes(&[seed; 32]).stick_id();
    device
        .store
        .add_joined_stick(&crate::store::JoinedStick {
            name: Some("Holiday".to_owned()),
            ..crate::store::JoinedStick::new(stick_id, output, 1)
        })
        .unwrap();
    stick_id
}

fn check_received(device: &Device, path: &Path, seed: u8) -> Result<(), PlaceError> {
    let stick_id = StickSigningKey::from_bytes(&[seed; 32]).stick_id();
    check_received_folder(&device.store, &device.data, path, &stick_id)
}

#[test]
fn a_received_stick_never_lands_where_it_would_be_shared_onward() {
    let device = device();
    // Inside a folder a command-line stick shares, or a linked folder.
    let inside_shared = device.disk.join("Downloads/Holiday");
    assert!(matches!(
        check_received(&device, &inside_shared, 9),
        Err(PlaceError::Shared { what, .. }) if what.contains("Rescued_Disk")
    ));
    assert!(check_received(&device, &device.documentaries.join("x"), 9).is_err());
    // Around a shared folder too.
    assert!(check_received(&device, device.disk.parent().unwrap(), 9).is_err());
    // An app stick's folder stays OurStick's alone.
    assert!(matches!(
        check_received(&device, &device.app.join("x"), 9),
        Err(PlaceError::Taken { .. })
    ));
    // Somewhere of its own is fine.
    let downloads = device.data.parent().unwrap().join("Downloads/Holiday");
    assert!(check_received(&device, &downloads, 9).is_ok());
}

#[test]
fn each_received_stick_has_a_folder_of_its_own() {
    let device = device();
    let downloads = device.data.parent().unwrap().join("Downloads");
    let first = received_into(&device, 9, &downloads.join("Holiday"));
    // Another stick may not share it, nor go inside or around it.
    for path in [
        downloads.join("Holiday"),
        downloads.join("Holiday/more"),
        downloads.clone(),
    ] {
        assert!(matches!(
            check_received(&device, &path, 10),
            Err(PlaceError::Shared { what, .. }) if what.contains("Holiday")
        ));
    }
    // The stick itself may keep its folder.
    let stick_id = first;
    assert!(check_received_folder(
        &device.store,
        &device.data,
        &downloads.join("Holiday"),
        &stick_id
    )
    .is_ok());
    assert!(check_received(&device, &downloads.join("Films"), 10).is_ok());
}

#[cfg(unix)]
#[test]
fn a_folder_not_made_yet_is_checked_where_its_links_lead() {
    let device = device();
    // ~/link points into a folder a command-line stick shares; ~/link/new
    // does not exist yet.
    let link = device.data.parent().unwrap().join("link");
    std::os::unix::fs::symlink(&device.documentaries, &link).unwrap();
    assert!(matches!(
        check_received(&device, &link.join("new"), 9),
        Err(PlaceError::Shared { .. })
    ));
}
