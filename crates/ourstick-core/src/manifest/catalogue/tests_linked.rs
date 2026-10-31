//! A catalogue over several roots (ADR 0008).

use super::*;
use crate::cipher::{ChaChaCipher, StickKey};
use crate::store::LinkedEntry;

fn cipher() -> ChaChaCipher {
    ChaChaCipher::new(&StickKey::from_bytes([3; 32]))
}

fn write(path: &Path, contents: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn linked(name: &str, path: &Path) -> LinkedEntry {
    let metadata = std::fs::metadata(path).unwrap();
    LinkedEntry {
        name: name.to_owned(),
        path: path.to_path_buf(),
        is_dir: metadata.is_dir(),
        disk: crate::linked::disk_of(&metadata),
        added_at_ms: 1,
        at_top: false,
    }
}

/// Every file of a catalogue run to its end, by path.
fn all(catalogue: &mut Catalogue) -> Vec<CataloguedFile> {
    let part = catalogue
        .next_part(&cipher(), usize::MAX, u64::MAX, None)
        .unwrap();
    part.into_iter()
        .map(|item| match item {
            Catalogued::File(file) => file,
            Catalogued::Skipped { path } => panic!("skipped {path}"),
        })
        .collect()
}

struct Disk {
    _dir: tempfile::TempDir,
    own: PathBuf,
    films: PathBuf,
    notes: PathBuf,
}

/// A stick folder with `a.txt` and `films/old.mkv`, a linked folder `Films`
/// with `x.mkv` and `2024/y.mkv` (and a symbolic link), a linked file.
fn disk() -> Disk {
    let dir = tempfile::tempdir().unwrap();
    let own = dir.path().join("stick");
    let films = dir.path().join("elsewhere/Films");
    let notes = dir.path().join("notes.txt");
    write(&own.join("a.txt"), b"a");
    write(&own.join("films/old.mkv"), b"old");
    write(&films.join("x.mkv"), b"x");
    write(&films.join("2024/y.mkv"), b"y");
    write(&notes, b"notes");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&own, films.join("into-the-stick")).unwrap();
    Disk {
        own,
        films,
        notes,
        _dir: dir,
    }
}

fn layout(disk: &Disk) -> StickLayout {
    StickLayout::new(
        &disk.own,
        &[
            linked("Films", &disk.films),
            linked("notes.txt", &disk.notes),
        ],
    )
}

#[test]
fn every_root_is_walked_under_its_name_and_links_inside_are_not_followed() {
    let disk = disk();
    let mut catalogue = Catalogue::start_layout(
        std::sync::Arc::new(crate::sender_files::LocalFiles),
        &layout(&disk),
        Vec::new(),
        false,
    )
    .unwrap();
    // The own "films" is hidden by the linked "Films", whatever the case.
    assert_eq!(catalogue.left_out(), ["films"]);
    assert!(catalogue.gone().is_empty());
    let paths: Vec<String> = all(&mut catalogue)
        .into_iter()
        .map(|file| file.entry.path)
        .collect();
    assert_eq!(
        paths,
        ["Films/2024/y.mkv", "Films/x.mkv", "a.txt", "notes.txt"]
    );
}

#[test]
fn a_gone_linked_entry_keeps_its_files_as_recorded_even_on_a_rescan() {
    let disk = disk();
    let layout = layout(&disk);
    let mut first = Catalogue::start_layout(
        std::sync::Arc::new(crate::sender_files::LocalFiles),
        &layout,
        Vec::new(),
        false,
    )
    .unwrap();
    let recorded: Vec<OwnedFile> = all(&mut first)
        .iter()
        .map(CataloguedFile::owned_file)
        .collect();
    let before: Vec<Entry> = all(&mut Catalogue::start_layout(
        std::sync::Arc::new(crate::sender_files::LocalFiles),
        &layout,
        recorded.clone(),
        false,
    )
    .unwrap())
    .into_iter()
    .map(|file| file.entry)
    .collect();

    // Its disk unplugged.
    std::fs::rename(&disk.films, disk.films.with_file_name("away")).unwrap();
    for rescan in [false, true] {
        let mut catalogue = Catalogue::start_layout(
            std::sync::Arc::new(crate::sender_files::LocalFiles),
            &layout,
            recorded.clone(),
            rescan,
        )
        .unwrap();
        assert_eq!(catalogue.gone(), ["Films"]);
        let files = all(&mut catalogue);
        let films: Vec<&CataloguedFile> = files
            .iter()
            .filter(|file| file.entry.path.starts_with("Films/"))
            .collect();
        assert_eq!(films.len(), 2);
        assert!(films.iter().all(|file| !file.read));
        // The very entries the manifest had.
        let entries: Vec<Entry> = files.into_iter().map(|file| file.entry).collect();
        assert_eq!(entries, before);
    }

    // Back: read as usual, nothing new.
    std::fs::rename(disk.films.with_file_name("away"), &disk.films).unwrap();
    let mut back = Catalogue::start_layout(
        std::sync::Arc::new(crate::sender_files::LocalFiles),
        &layout,
        recorded,
        false,
    )
    .unwrap();
    assert!(back.gone().is_empty());
    let entries: Vec<Entry> = all(&mut back).into_iter().map(|file| file.entry).collect();
    assert_eq!(entries, before);
}

#[test]
fn a_linked_file_is_one_entry_under_its_name() {
    let disk = disk();
    let layout = StickLayout::new(&disk.own, &[linked("notes.txt", &disk.notes)]);
    let files = all(&mut Catalogue::start_layout(
        std::sync::Arc::new(crate::sender_files::LocalFiles),
        &layout,
        Vec::new(),
        false,
    )
    .unwrap());
    let notes = files
        .iter()
        .find(|file| file.entry.path == "notes.txt")
        .unwrap();
    assert_eq!(notes.entry.size, 5);
    // No absolute path anywhere in what goes into a manifest.
    let manifest = crate::manifest::Manifest {
        partial: false,
        name: "s".to_owned(),
        entries: files.iter().map(|file| file.entry.clone()).collect(),
    };
    let bytes = manifest.to_bytes().unwrap();
    let place = disk.notes.parent().unwrap().to_string_lossy().into_owned();
    assert!(!String::from_utf8_lossy(&bytes).contains(&place));
}

/// A stick made from one folder ("Holiday" with `Beach/x.jpg` and
/// `notes.txt`), at its top, and an empty folder of its own.
fn at_top(disk: &Disk) -> (PathBuf, LinkedEntry) {
    let holiday = disk.own.with_file_name("Holiday");
    write(&holiday.join("Beach/x.jpg"), b"x");
    write(&holiday.join("notes.txt"), b"n");
    let empty = disk.own.with_file_name("sent");
    std::fs::create_dir_all(&empty).unwrap();
    let top = LinkedEntry {
        at_top: true,
        ..linked("Holiday", &holiday)
    };
    (empty, top)
}

#[test]
fn the_folder_at_the_top_gives_its_contents_without_its_name() {
    let disk = disk();
    let (empty, top) = at_top(&disk);
    let layout = StickLayout::new(&empty, &[top.clone(), linked("Films", &disk.films)]);
    let mut catalogue = Catalogue::start_layout(
        std::sync::Arc::new(crate::sender_files::LocalFiles),
        &layout,
        Vec::new(),
        false,
    )
    .unwrap();
    assert!(catalogue.left_out().is_empty());
    let paths: Vec<String> = all(&mut catalogue)
        .into_iter()
        .map(|file| file.entry.path)
        .collect();
    assert_eq!(
        paths,
        [
            "Beach/x.jpg",
            "Films/2024/y.mkv",
            "Films/x.mkv",
            "notes.txt"
        ]
    );
    assert_eq!(
        layout.on_disk("Beach/x.jpg"),
        top.path.join("Beach").join("x.jpg")
    );
    assert_eq!(layout.on_disk("Films/x.mkv"), disk.films.join("x.mkv"));

    // A name it gains later that a named entry has is not sent.
    write(&top.path.join("films/z.mkv"), b"z");
    let mut catalogue = Catalogue::start_layout(
        std::sync::Arc::new(crate::sender_files::LocalFiles),
        &layout,
        Vec::new(),
        false,
    )
    .unwrap();
    assert_eq!(catalogue.left_out(), ["films"]);
    assert_eq!(all(&mut catalogue).len(), 4);
}

#[test]
fn the_folder_at_the_top_gone_keeps_its_files_as_recorded() {
    let disk = disk();
    let (empty, top) = at_top(&disk);
    let layout = StickLayout::new(&empty, &[top.clone(), linked("Films", &disk.films)]);
    let recorded: Vec<OwnedFile> = all(&mut Catalogue::start_layout(
        std::sync::Arc::new(crate::sender_files::LocalFiles),
        &layout,
        Vec::new(),
        false,
    )
    .unwrap())
    .iter()
    .map(CataloguedFile::owned_file)
    .collect();
    std::fs::rename(&top.path, top.path.with_file_name("away")).unwrap();
    let mut catalogue = Catalogue::start_layout(
        std::sync::Arc::new(crate::sender_files::LocalFiles),
        &layout,
        recorded,
        false,
    )
    .unwrap();
    assert_eq!(catalogue.gone(), ["Holiday"]);
    // Its files as recorded, the named entry's as usual: the same manifest.
    let paths: Vec<String> = all(&mut catalogue)
        .into_iter()
        .map(|file| file.entry.path)
        .collect();
    assert_eq!(
        paths,
        [
            "Beach/x.jpg",
            "Films/2024/y.mkv",
            "Films/x.mkv",
            "notes.txt"
        ]
    );
}
