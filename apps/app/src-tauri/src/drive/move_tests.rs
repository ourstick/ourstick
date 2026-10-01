//! Moving the first stick of an older version into `~/OurStick`, and
//! finishing a move stopped at each point.

use std::path::{Path, PathBuf};

use ourstick_core::share::mark_drive;
use ourstick_core::{OwnedStick, StickSigningKey};

use super::{find_moved_in, move_into, moving_place};

/// A fresh folder standing for the home folder, removed when dropped.
struct Home(PathBuf);

impl Home {
    fn new(test: &str) -> Self {
        let place =
            std::env::temp_dir().join(format!("ourstick-move-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&place);
        std::fs::create_dir_all(&place).unwrap();
        Self(place)
    }

    fn sticks(&self) -> PathBuf {
        self.0.join("OurStick")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The first stick, `~/OurStick` itself, marked and holding a file.
fn first_stick(home: &Home) -> OwnedStick {
    let root = home.sticks();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("photo.jpg"), b"photo").unwrap();
    let id = StickSigningKey::from_bytes(&[3; 32]).stick_id();
    mark_drive(&root, &id).unwrap();
    OwnedStick::new(id, "OurStick", &root, ourstick_core::CipherId::None, 0)
}

fn photo_in(root: &Path) -> bool {
    std::fs::read(root.join("photo.jpg")).is_ok_and(|bytes| bytes == b"photo")
}

#[test]
fn the_first_stick_moves_into_its_own_folder_with_its_files() {
    let home = Home::new("whole");
    first_stick(&home);
    let root = move_into(&home.sticks(), "OurStick").unwrap();
    assert_eq!(root, home.sticks().join("OurStick"));
    assert!(photo_in(&root));
    assert!(!moving_place(&home.sticks()).exists());
}

#[test]
fn a_folder_in_the_way_stops_the_move_before_anything_moves() {
    let home = Home::new("in-the-way");
    first_stick(&home);
    std::fs::create_dir(moving_place(&home.sticks())).unwrap();
    let error = move_into(&home.sticks(), "OurStick").unwrap_err();
    assert!(format!("{error:#}").contains("in the way"));
    assert!(photo_in(&home.sticks()));
}

#[test]
fn a_move_stopped_after_the_first_rename_is_finished() {
    let home = Home::new("first-rename");
    let stick = first_stick(&home);
    std::fs::rename(home.sticks(), moving_place(&home.sticks())).unwrap();
    let root = find_moved_in(&home.sticks(), &stick, "OurStick").unwrap();
    assert_eq!(root, Some(home.sticks().join("OurStick")));
    assert!(photo_in(&home.sticks().join("OurStick")));
}

#[test]
fn a_move_stopped_after_the_new_folder_is_made_is_finished() {
    let home = Home::new("new-folder");
    let stick = first_stick(&home);
    std::fs::rename(home.sticks(), moving_place(&home.sticks())).unwrap();
    std::fs::create_dir(home.sticks()).unwrap();
    let root = find_moved_in(&home.sticks(), &stick, "OurStick").unwrap();
    assert_eq!(root, Some(home.sticks().join("OurStick")));
    assert!(photo_in(&home.sticks().join("OurStick")));
}

#[test]
fn a_finished_move_not_yet_recorded_is_found() {
    let home = Home::new("not-recorded");
    let stick = first_stick(&home);
    move_into(&home.sticks(), "OurStick").unwrap();
    let root = find_moved_in(&home.sticks(), &stick, "OurStick").unwrap();
    assert_eq!(root, Some(home.sticks().join("OurStick")));
}

#[test]
fn a_stick_where_it_is_recorded_or_nowhere_is_left_alone() {
    let home = Home::new("left-alone");
    let stick = first_stick(&home);
    assert_eq!(
        find_moved_in(&home.sticks(), &stick, "OurStick").unwrap(),
        None
    );
    std::fs::remove_dir_all(home.sticks()).unwrap();
    assert_eq!(
        find_moved_in(&home.sticks(), &stick, "OurStick").unwrap(),
        None
    );
}
