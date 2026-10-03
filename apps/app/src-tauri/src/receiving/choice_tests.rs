//! Choosing what to get, as the window lists and saves it.

use super::*;
use ourstick_core::store::{DownloadedFile, JoinedStick, MemoryStore};

fn manifest(files: &[(&str, u64)]) -> Manifest {
    Manifest {
        entries: files
            .iter()
            .map(|(path, size)| ourstick_core::manifest::Entry {
                path: (*path).to_owned(),
                size: *size,
                mtime: 0,
                hash: [0; 32],
            })
            .collect(),
        ..Manifest::default()
    }
}

fn downloaded(path: &str) -> DownloadedFile {
    DownloadedFile {
        path: path.to_owned(),
        saved_as: path.to_owned(),
        stamp: ourstick_core::manifest::FileStamp {
            size: 1,
            mtime_ns: 1,
            ctime_ns: 1,
        },
        blob_hash: [1; 32],
    }
}

fn received(store: &MemoryStore, seed: u8) -> StickId {
    let id = ourstick_core::StickSigningKey::from_bytes(&[seed; 32]).stick_id();
    store
        .add_joined_stick(&JoinedStick::new(id, "/d/x", 1))
        .unwrap();
    id
}

#[test]
fn a_choice_is_saved_whole_and_the_stick_goes_on_unless_paused() {
    let store = MemoryStore::new();
    let id = received(&store, 5);
    let folder = save_choice(&store, &id, &["Films".to_owned()]).unwrap();
    assert_eq!(folder.as_deref(), Some(std::path::Path::new("/d/x")));
    assert_eq!(store.left_out(&id).unwrap(), ["Films"]);
    // Paused: saved, and nothing starts.
    store
        .change_joined_stick(&id, &ourstick_core::store::JoinedChange::Paused(true))
        .unwrap();
    assert_eq!(save_choice(&store, &id, &[]).unwrap(), None);
    assert!(store.left_out(&id).unwrap().is_empty());
    // A path inside a folder is saved (ADR 0012, Amendment 1).
    save_choice(&store, &id, &["Disk/MUSIC".to_owned()]).unwrap();
    assert_eq!(store.left_out(&id).unwrap(), ["Disk/MUSIC"]);
    // Not plain paths: refused, the last choice kept.
    store.set_left_out(&id, &["Music".to_owned()]).unwrap();
    for bad in ["", "a/b/c", "a/..", "/a"] {
        assert!(save_choice(&store, &id, &[bad.to_owned()]).is_err());
    }
    assert_eq!(store.left_out(&id).unwrap(), ["Music"]);
    // A stick not received here: refused.
    let other = ourstick_core::StickSigningKey::from_bytes(&[6; 32]).stick_id();
    assert!(save_choice(&store, &other, &[]).is_err());
}

#[test]
fn the_list_says_what_is_left_out_and_whether_it_is_here() {
    let store = MemoryStore::new();
    let id = received(&store, 7);
    assert!(left_out_views(&store, &id).unwrap().is_empty());
    store
        .set_left_out(
            &id,
            &[
                "Films".to_owned(),
                "a\u{202e}".to_owned(),
                "Disk/MUSIC".to_owned(),
            ],
        )
        .unwrap();
    store
        .record_downloaded_file(&id, &downloaded("Films/x"))
        .unwrap();
    assert_eq!(
        left_out_views(&store, &id).unwrap(),
        [
            LeftOutView {
                shown: "MUSIC".to_owned(),
                here: false,
                inside: Some("Disk".to_owned()),
            },
            LeftOutView {
                shown: "Films".to_owned(),
                here: true,
                inside: None,
            },
            LeftOutView {
                shown: "a\\u{202e}".to_owned(),
                here: false,
                inside: None,
            },
        ]
    );
}

#[test]
fn the_choice_lists_every_entry_and_every_name_left_out() {
    let store = MemoryStore::new();
    let id = received(&store, 4);
    store
        .set_left_out(&id, &["Films".to_owned(), "Gone".to_owned()])
        .unwrap();
    store
        .record_downloaded_file(&id, &downloaded("Films/a.mkv"))
        .unwrap();
    let stick = manifest(&[("Music/m", 5), ("Films/a.mkv", 9), ("a\u{202e}b", 1)]);
    let entries = choice_entries(&store, &id, Some(&stick), None).unwrap();
    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    // Largest first; a name left out that the stick no longer has, last.
    assert_eq!(names, ["Films", "Music", "a\u{202e}b", "Gone"]);
    assert!(entries[0].tick == "off" && entries[0].here && entries[0].on_stick);
    assert!(entries[1].tick == "on" && !entries[1].here);
    assert_eq!(entries[2].shown, "a\\u{202e}b");
    assert!(entries[3].tick == "off" && !entries[3].on_stick);
    // Before any round: only what is left out.
    let before = choice_entries(&store, &id, None, None).unwrap();
    assert_eq!(before.len(), 2);
}

#[test]
fn inside_a_folder_each_entry_is_named_by_its_path_and_ticked() {
    let store = MemoryStore::new();
    let id = received(&store, 8);
    // One inside left out, and one the stick no longer has.
    store
        .set_left_out(&id, &["Disk/MUSIC".to_owned(), "Disk/Gone".to_owned()])
        .unwrap();
    let stick = manifest(&[
        ("Disk/MUSIC/a", 4),
        ("Disk/Pictures/p", 3),
        ("Disk/readme", 1),
    ]);
    let top = choice_entries(&store, &id, Some(&stick), None).unwrap();
    assert_eq!(top.len(), 1);
    assert_eq!((top[0].name.as_str(), top[0].tick), ("Disk", "part"));
    let inside = choice_entries(&store, &id, Some(&stick), Some("Disk")).unwrap();
    let named: Vec<(&str, &str, &str, bool)> = inside
        .iter()
        .map(|entry| {
            (
                entry.name.as_str(),
                entry.shown.as_str(),
                entry.tick,
                entry.on_stick,
            )
        })
        .collect();
    assert_eq!(
        named,
        [
            ("Disk/MUSIC", "MUSIC", "off", true),
            ("Disk/Pictures", "Pictures", "on", true),
            ("Disk/readme", "readme", "on", true),
            ("Disk/Gone", "Gone", "off", false),
        ]
    );
}

#[test]
fn what_is_left_out_and_here_is_told_also_unopened() {
    let store = MemoryStore::new();
    let id = received(&store, 9);
    store
        .record_downloaded_file(&id, &downloaded("Disk/MUSIC/a"))
        .unwrap();
    let left_out = ["Disk/MUSIC".to_owned(), "Films".to_owned()];
    assert_eq!(
        staying(&store, &id, &left_out).unwrap(),
        [Staying {
            name: "Disk/MUSIC".to_owned(),
            shown: "MUSIC".to_owned(),
        }]
    );
}
