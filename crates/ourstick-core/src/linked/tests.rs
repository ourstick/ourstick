use super::*;
use crate::cipher::CipherId;
use crate::stick_id::StickSigningKey;
use crate::store::{MemoryStore, OwnedStick, StickKind};

struct Setup {
    _dir: tempfile::TempDir,
    own: PathBuf,
    data: PathBuf,
    outside: PathBuf,
    store: MemoryStore,
    stick_id: StickId,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let own = dir.path().join("OurStick");
    let data = dir.path().join("data");
    let outside = dir.path().join("disk");
    for folder in [&own, &data, &outside] {
        std::fs::create_dir(folder).unwrap();
    }
    let store = MemoryStore::new();
    let stick_id = StickSigningKey::from_bytes(&[1; 32]).stick_id();
    store
        .add_owned_stick(&OwnedStick {
            kind: StickKind::App,
            ..OwnedStick::new(stick_id, "s", &own, CipherId::XChaCha20Poly1305, 1)
        })
        .unwrap();
    Setup {
        own,
        data,
        outside,
        store,
        stick_id,
        _dir: dir,
    }
}

/// Plans a link with `planned` decided but not stored yet.
fn plan(setup: &Setup, planned: &[LinkedEntry], path: &Path) -> Result<LinkedEntry, LinkError> {
    plan_link(&setup.store, &setup.stick_id, &setup.data, planned, path, 5)
}

#[test]
fn a_folder_or_a_file_outside_is_linked_under_its_name() {
    let setup = setup();
    let films = setup.outside.join("Films");
    std::fs::create_dir(&films).unwrap();
    let entry = plan(&setup, &[], &films).unwrap();
    assert_eq!(entry.name, "Films");
    assert_eq!(entry.path, films);
    assert!(entry.is_dir);
    assert_eq!(entry.added_at_ms, 5);
    #[cfg(unix)]
    assert!(entry.disk.is_some());

    let notes = setup.outside.join("notes.txt");
    std::fs::write(&notes, "x").unwrap();
    let entry = plan(&setup, &[], &notes).unwrap();
    assert_eq!((entry.name.as_str(), entry.is_dir), ("notes.txt", false));
}

#[test]
fn a_name_on_the_stick_already_gets_a_number_ignoring_case() {
    let setup = setup();
    std::fs::create_dir(setup.own.join("films")).unwrap();
    let films = setup.outside.join("Films");
    std::fs::create_dir(&films).unwrap();
    let first = plan(&setup, &[], &films).unwrap();
    assert_eq!(first.name, "Films (2)");
    // Another linked "FILMS (2)" takes that one too.
    let other = setup.outside.join("x").join("Films");
    std::fs::create_dir_all(&other).unwrap();
    let taken = LinkedEntry {
        name: "FILMS (2)".to_owned(),
        ..first
    };
    assert_eq!(plan(&setup, &[taken], &other).unwrap().name, "Films (3)");
    // Greek too: case is Unicode's, not ASCII's.
    std::fs::create_dir(setup.own.join("ΤΑΙΝΙΕΣ")).unwrap();
    let greek = setup.outside.join("ταινιες");
    std::fs::create_dir(&greek).unwrap();
    assert_eq!(plan(&setup, &[], &greek).unwrap().name, "ταινιες (2)");
}

#[test]
fn overlaps_are_refused() {
    let setup = setup();
    let films = setup.outside.join("Films");
    std::fs::create_dir_all(films.join("2024")).unwrap();
    let linked = plan(&setup, &[], &films).unwrap();
    let existing = [linked];
    let refused = |path: &Path, existing: &[LinkedEntry]| {
        matches!(
            plan(&setup, existing, path),
            Err(LinkError::Overlaps { .. })
        )
    };
    // The same, inside, containing.
    assert!(refused(&films, &existing));
    assert!(refused(&films.join("2024"), &existing));
    assert!(refused(&setup.outside, &existing));
    // The stick's own folder, something in it, a folder holding it.
    std::fs::create_dir(setup.own.join("inside")).unwrap();
    assert!(refused(&setup.own, &[]));
    assert!(refused(&setup.own.join("inside"), &[]));
    assert!(refused(setup.own.parent().unwrap(), &[]));
    // The data folder (and the cache in it).
    std::fs::create_dir(setup.data.join("cache")).unwrap();
    assert!(refused(&setup.data, &[]));
    assert!(refused(&setup.data.join("cache"), &[]));
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_is_refused_and_one_into_the_stick_cannot_sneak_in() {
    let setup = setup();
    let link = setup.outside.join("shortcut");
    std::os::unix::fs::symlink(&setup.own, &link).unwrap();
    assert!(matches!(
        plan(&setup, &[], &link),
        Err(LinkError::SymbolicLink(_))
    ));
    // A folder reached through a linked parent resolves to the stick.
    let via = setup.outside.join("via");
    std::os::unix::fs::symlink(setup.own.parent().unwrap(), &via).unwrap();
    assert!(matches!(
        plan(&setup, &[], &via.join("OurStick")),
        Err(LinkError::Overlaps { .. })
    ));
}

#[test]
fn a_relative_or_missing_path_is_refused() {
    let setup = setup();
    assert!(matches!(
        plan(&setup, &[], Path::new("Films")),
        Err(LinkError::NotAbsolute(_))
    ));
    assert!(matches!(
        plan(&setup, &[], &setup.outside.join("nothing")),
        Err(LinkError::Io { .. })
    ));
}

#[test]
fn another_app_sticks_folder_is_refused_but_a_command_line_ones_is_not() {
    let setup = setup();
    let other_app = setup.outside.join("For Maria");
    let cli = setup.outside.join("Shared from the command line");
    std::fs::create_dir_all(other_app.join("inside")).unwrap();
    std::fs::create_dir_all(&cli).unwrap();
    for (seed, root, kind) in [(2, &other_app, StickKind::App), (3, &cli, StickKind::Cli)] {
        setup
            .store
            .add_owned_stick(&OwnedStick {
                kind,
                ..OwnedStick::new(
                    StickSigningKey::from_bytes(&[seed; 32]).stick_id(),
                    "other",
                    root,
                    CipherId::XChaCha20Poly1305,
                    1,
                )
            })
            .unwrap();
    }
    for refused in [&other_app, &other_app.join("inside"), &setup.outside] {
        assert!(
            matches!(plan(&setup, &[], refused), Err(LinkError::Overlaps { .. })),
            "{}",
            refused.display()
        );
    }
    // The same files in two sticks: one per person.
    assert_eq!(
        plan(&setup, &[], &cli).unwrap().name,
        "Shared from the command line"
    );
}
