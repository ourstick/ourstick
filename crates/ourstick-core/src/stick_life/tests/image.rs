//! macOS: an older app stick is its disk image, which must be known before
//! anything of it is deleted (`image_known`). The rule itself is tested on
//! every platform; what it does to a removal, on macOS.

use super::*;

#[test]
fn an_image_matters_only_for_a_drive_that_is_there() {
    let world = world();
    let stick = unrecorded_app_stick(&world, "For Maria");
    assert!(!image_known_where(true, &stick), "there, image unknown");
    assert!(image_known_where(false, &stick), "not macOS");
    let recorded = OwnedStick {
        image_path: Some(PathBuf::from("/images/For Maria.sparseimage")),
        ..stick.clone()
    };
    assert!(image_known_where(true, &recorded));
    // Not there (its image deleted, never mounted): nothing to touch.
    std::fs::rename(&stick.root_path, stick.root_path.with_file_name("away")).unwrap();
    assert!(image_known_where(true, &stick));
}

#[test]
fn a_new_link_keeps_the_stick_s_image() {
    let world = world();
    let old = unrecorded_app_stick(&world, "For Maria");
    let image = PathBuf::from("/images/For Maria.sparseimage");
    world.store.set_image_path(&old.stick_id, &image).unwrap();
    let new = give_new_link(&world.store, &world.secrets, &world.data, &old.stick_id).unwrap();
    assert_eq!(new.stick.image_path.as_deref(), Some(image.as_path()));
    let stored = world
        .store
        .owned_stick(&new.stick.stick_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.image_path, Some(image));
}

/// Refused before it is marked: nothing waits for the next start.
#[cfg(target_os = "macos")]
#[test]
fn a_stick_whose_disk_image_is_not_recorded_is_left_as_it_was() {
    let world = world();
    let going = unrecorded_app_stick(&world, "For Maria");
    let refused = remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &going.stick_id,
        &NoDrive,
    );
    assert!(
        matches!(refused, Err(LifeError::ImageNotRecorded)),
        "{refused:?}"
    );
    assert!(world.store.removing_sticks().unwrap().is_empty());
    assert_eq!(world.store.owned_sticks().unwrap(), [going]);
}

/// One already being removed waits for the stick to be ejected (its image
/// can no longer be recorded): its keys and cache stay.
#[cfg(target_os = "macos")]
#[test]
fn a_marked_removal_waits_for_the_stick_to_be_ejected() {
    let world = world();
    let going = unrecorded_app_stick(&world, "For Maria");
    world.store.mark_removing(&going.stick_id).unwrap();
    let failed = finish_removals(&world.store, &world.secrets, &world.data, &NoDrive).unwrap();
    assert!(
        matches!(failed.as_slice(), [(_, LifeError::Drive(_))]),
        "{failed:?}"
    );
    assert!(has_secret(&world, SecretName::StickKey(going.stick_id)));
    assert!(cache_dir(&world.data, &going.stick_id).exists());
    assert_eq!(world.store.removing_sticks().unwrap().len(), 1);
}

/// Its image deleted and never recorded: it can still be removed, and its
/// folder, not there, is not touched.
#[cfg(target_os = "macos")]
#[test]
fn a_stick_whose_image_is_gone_can_still_be_removed() {
    let world = world();
    let going = unrecorded_app_stick(&world, "For Maria");
    let away = going.root_path.with_file_name("away");
    std::fs::rename(&going.root_path, &away).unwrap();
    remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &going.stick_id,
        &NoDrive,
    )
    .unwrap();
    assert!(world.store.owned_sticks().unwrap().is_empty());
    assert!(away.join("photos/a.jpg").exists());
}

/// Another stick's volume at its place, its image unrecorded: removed, and
/// the other's marker is left alone.
#[cfg(target_os = "macos")]
#[test]
fn another_stick_at_its_place_is_left_alone() {
    let world = world();
    let going = unrecorded_app_stick(&world, "For Maria");
    let other = StickSigningKey::from_bytes(&[9; 32]).stick_id();
    mark_drive(&going.root_path, &other).unwrap();
    remove_stick(
        &world.store,
        &world.secrets,
        &world.data,
        &going.stick_id,
        &NoDrive,
    )
    .unwrap();
    assert!(crate::share::drive_marked_by(&going.root_path, &other));
}
