//! Windows tests of [`super::open_regular`]: links, junctions and held
//! folders.

use super::*;

/// A junction (no rights needed to make one, unlike a link).
fn junction(at: &Path, to: &Path) {
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(at)
        .arg(to)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
}

fn folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("root/sub")).unwrap();
    std::fs::write(dir.path().join("root/sub/a.txt"), b"in the stick").unwrap();
    std::fs::create_dir(dir.path().join("outside")).unwrap();
    std::fs::write(dir.path().join("outside/a.txt"), b"not in the stick").unwrap();
    dir
}

#[test]
fn a_file_under_its_root_is_read_and_the_root_may_be_a_junction() {
    let dir = folder();
    let path = dir.path().join("root/sub/a.txt");
    let file = open_regular(&path, 2).unwrap();
    assert_eq!(std::io::read_to_string(file).unwrap(), "in the stick");
    junction(&dir.path().join("chosen"), &dir.path().join("root"));
    assert!(open_regular(&dir.path().join("chosen/sub/a.txt"), 2).is_ok());
    assert!(open_regular(&path, 0).is_ok());
}

#[test]
fn a_junction_in_place_of_a_folder_is_refused() {
    let dir = folder();
    let sub = dir.path().join("root").join("sub");
    std::fs::rename(&sub, dir.path().join("old")).unwrap();
    junction(&sub, &dir.path().join("outside"));
    let error = open_regular(&dir.path().join("root/sub/a.txt"), 2).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn a_folder_on_the_way_cannot_be_renamed_while_it_is_held() {
    let dir = folder();
    use std::os::windows::fs::OpenOptionsExt;
    // Held as open_regular holds it, between the check and the open.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x0200_0000 | 0x0020_0000)
        .share_mode(0x1 | 0x2)
        .open(dir.path().join("root/sub"))
        .unwrap();
    let moved = std::fs::rename(dir.path().join("root/sub"), dir.path().join("old"));
    assert!(moved.is_err(), "renamed while held");
    drop(held);
    assert!(std::fs::rename(dir.path().join("root/sub"), dir.path().join("old")).is_ok());
}

#[test]
fn a_held_folder_that_is_a_junction_now_is_refused() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = folder();
    let via = dir.path().join("root").join("via");
    junction(&via, &dir.path().join("outside"));
    // Opened as open_regular opens a folder on the way: as itself.
    let as_itself = |path: &Path| {
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .share_mode(0x1 | 0x2)
            .open(path)
            .unwrap()
    };
    let plain = as_itself(&dir.path().join("root").join("sub"));
    assert!(still_not_links(std::slice::from_ref(&plain)).is_ok());
    let turned = as_itself(&via);
    let error = still_not_links(&[plain, turned]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

/// Another program holding a folder open for deleting (a sync app) keeps
/// it from being held: the file is still read, by the fallback.
#[test]
fn a_folder_held_elsewhere_for_deleting_is_still_read() {
    use std::os::windows::fs::OpenOptionsExt;
    const DELETE: u32 = 0x0001_0000;
    let dir = folder();
    let elsewhere = std::fs::OpenOptions::new()
        .access_mode(DELETE)
        .custom_flags(0x0200_0000)
        .share_mode(0x1 | 0x2 | 0x4)
        .open(dir.path().join("root").join("sub"))
        .unwrap();
    let file = open_regular(&dir.path().join("root/sub/a.txt"), 2).unwrap();
    assert_eq!(std::io::read_to_string(file).unwrap(), "in the stick");
    drop(elsewhere);
}

#[test]
fn a_folder_is_not_a_file() {
    let dir = folder();
    let error = open_regular(&dir.path().join("root/sub"), 1).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn a_link_in_place_of_the_file_is_refused() {
    let dir = folder();
    let file = dir.path().join("root").join("sub").join("a.txt");
    std::fs::remove_file(&file).unwrap();
    // A link to a file needs Developer Mode or an administrator.
    let outside = dir.path().join("outside").join("a.txt");
    if std::os::windows::fs::symlink_file(&outside, &file).is_err() {
        return;
    }
    let error = open_regular(&file, 2).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}
