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
    }
}

/// Every file of a catalogue run to its end, by path.
fn all(catalogue: &mut Catalogue) -> Vec<CataloguedFile> {
    let part = catalogue
        .next_part(&cipher(), usize::MAX, u64::MAX)
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
    let mut catalogue = Catalogue::start_layout(&layout(&disk), Vec::new(), false).unwrap();
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
    let mut first = Catalogue::start_layout(&layout, Vec::new(), false).unwrap();
    let recorded: Vec<OwnedFile> = all(&mut first)
        .iter()
        .map(CataloguedFile::owned_file)
        .collect();
    let before: Vec<Entry> =
        all(&mut Catalogue::start_layout(&layout, recorded.clone(), false).unwrap())
            .into_iter()
            .map(|file| file.entry)
            .collect();

    // Its disk unplugged.
    std::fs::rename(&disk.films, disk.films.with_file_name("away")).unwrap();
    for rescan in [false, true] {
        let mut catalogue = Catalogue::start_layout(&layout, recorded.clone(), rescan).unwrap();
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
    let mut back = Catalogue::start_layout(&layout, recorded, false).unwrap();
    assert!(back.gone().is_empty());
    let entries: Vec<Entry> = all(&mut back).into_iter().map(|file| file.entry).collect();
    assert_eq!(entries, before);
}

#[test]
fn a_linked_file_is_one_entry_under_its_name() {
    let disk = disk();
    let layout = StickLayout::new(&disk.own, &[linked("notes.txt", &disk.notes)]);
    let files = all(&mut Catalogue::start_layout(&layout, Vec::new(), false).unwrap());
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
