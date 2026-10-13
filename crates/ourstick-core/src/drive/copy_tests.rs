use super::*;

fn write(path: &Path, bytes: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![7u8; bytes]).unwrap();
}

#[test]
fn names_already_on_the_stick_are_told_before_copying() {
    let stick = tempfile::tempdir().unwrap();
    let from = tempfile::tempdir().unwrap();
    write(&stick.path().join("new in/a.jpg"), 10);
    for name in ["new in/b.jpg", "Films/c.mkv", "notes.txt"] {
        write(&from.path().join(name), 10);
    }
    let sources: Vec<PathBuf> = ["new in", "Films", "notes.txt"]
        .iter()
        .map(|name| from.path().join(name))
        .collect();
    // Only the copy there is asked about; a linked entry's name is not.
    assert_eq!(names_taken(stick.path(), &sources), ["new in"]);
}

#[test]
fn replace_takes_the_copys_place_and_never_a_linked_entrys() {
    let stick = tempfile::tempdir().unwrap();
    let from = tempfile::tempdir().unwrap();
    write(&stick.path().join("new in/old.jpg"), 50);
    write(&from.path().join("new in/new.jpg"), 60);
    write(&from.path().join("Films/c.mkv"), 5);
    let sources = [from.path().join("new in"), from.path().join("Films")];
    // 65 bytes needed: both are there until the new one is whole.
    let names = copy_onto(
        stick.path(),
        65,
        &["Films".to_owned()],
        &sources,
        OnConflict::Replace,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(names, ["new in", "Films (2)"]);
    assert!(!stick.path().join("new in/old.jpg").exists());
    assert!(stick.path().join("new in/new.jpg").exists());
    // The originals are where they were.
    assert!(from.path().join("new in/new.jpg").exists());
}

#[test]
fn the_bytes_are_told_as_they_are_copied() {
    let stick = tempfile::tempdir().unwrap();
    let from = tempfile::tempdir().unwrap();
    write(&from.path().join("big.bin"), 3 * PIECE + 5);
    write(&from.path().join("folder/small.txt"), 10);
    let mut told = Vec::new();
    copy_onto(
        stick.path(),
        u64::MAX,
        &[],
        &[from.path().join("big.bin"), from.path().join("folder")],
        OnConflict::KeepBoth,
        &mut |copied| told.push(copied),
    )
    .unwrap();
    let total = (3 * PIECE + 15) as u64;
    assert!(told.iter().all(|copied| copied.total == total));
    assert!(told.windows(2).all(|pair| pair[0].done < pair[1].done));
    assert_eq!(told.last().unwrap().done, total);
    assert!(told.len() >= 5);
}

#[cfg(unix)]
#[test]
fn a_source_on_the_stick_through_a_link_is_refused_and_nothing_goes() {
    let stick = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    write(&stick.path().join("Photos/a.jpg"), 10);
    // The stick's own folder, seen through a link (as a drive letter is).
    let through = elsewhere.path().join("stick");
    std::os::unix::fs::symlink(stick.path(), &through).unwrap();
    let source = through.join("Photos");
    let refused = copy_onto(
        stick.path(),
        u64::MAX,
        &[],
        std::slice::from_ref(&source),
        OnConflict::Replace,
        &mut |_| {},
    );
    assert!(matches!(refused, Err(DriveError::NotCopyable { .. })));
    assert!(check_sources(stick.path(), &[source]).is_err());
    // A folder that holds the stick is refused too.
    assert!(check_sources(stick.path(), &[elsewhere.path().to_path_buf()]).is_ok());
    assert!(check_sources(&stick.path().join("Photos"), &[stick.path().to_path_buf()]).is_err());
    assert!(stick.path().join("Photos/a.jpg").exists());
}

#[cfg(unix)]
#[test]
fn a_replace_that_fails_keeps_the_old_copy_and_leaves_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let stick = tempfile::tempdir().unwrap();
    let from = tempfile::tempdir().unwrap();
    write(&stick.path().join("new in/old.jpg"), 50);
    write(&from.path().join("new in/good.jpg"), 10);
    write(&from.path().join("new in/locked.jpg"), 10);
    let locked = from.path().join("new in/locked.jpg");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&locked).is_ok() {
        return; // run as root: nothing is unreadable
    }
    let failed = copy_onto(
        stick.path(),
        u64::MAX,
        &[],
        &[from.path().join("new in")],
        OnConflict::Replace,
        &mut |_| {},
    );
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(failed.is_err());
    assert!(stick.path().join("new in/old.jpg").exists());
    let names: Vec<String> = std::fs::read_dir(stick.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["new in"], "nothing half done is left");
}

#[test]
fn keep_both_numbers_the_new_one_and_times_are_kept() {
    let stick = tempfile::tempdir().unwrap();
    let from = tempfile::tempdir().unwrap();
    write(&stick.path().join("notes.txt"), 5);
    write(&from.path().join("notes.txt"), 7);
    let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    std::fs::File::options()
        .write(true)
        .open(from.path().join("notes.txt"))
        .unwrap()
        .set_modified(when)
        .unwrap();
    // What a copy cut short left behind goes first.
    write(&stick.path().join(".ourstick-copying-9"), 3);
    let names = copy_onto(
        stick.path(),
        u64::MAX,
        &[],
        &[from.path().join("notes.txt")],
        OnConflict::KeepBoth,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(names, ["notes (2).txt"]);
    assert_eq!(
        std::fs::read(stick.path().join("notes.txt")).unwrap().len(),
        5
    );
    let copy = std::fs::metadata(stick.path().join("notes (2).txt")).unwrap();
    assert_eq!(copy.len(), 7);
    assert_eq!(copy.modified().unwrap(), when);
    assert!(!stick.path().join(".ourstick-copying-9").exists());
}

#[test]
fn a_drop_is_looked_at_first_and_said_so_then_copied() {
    let stick = tempfile::tempdir().unwrap();
    let from = tempfile::tempdir().unwrap();
    write(&from.path().join("folder/a.bin"), 10);
    write(&from.path().join("folder/b.bin"), 20);
    write(&from.path().join("folder/.DS_Store"), 99);
    let mut steps = Vec::new();
    copy_onto_stoppable(
        stick.path(),
        u64::MAX,
        &[],
        &[from.path().join("folder")],
        OnConflict::KeepBoth,
        &AtomicBool::new(false),
        &mut |step| steps.push(step),
    )
    .unwrap();
    // Counted file by file (system files left out, as they are not copied)…
    let measured: Vec<_> = steps
        .iter()
        .filter_map(|step| match step {
            CopyStep::Measuring { files, bytes } => Some((*files, *bytes)),
            CopyStep::Copying(_) => None,
        })
        .collect();
    assert_eq!(measured.len(), 2);
    assert_eq!(measured.last(), Some(&(2, 30)));
    // …and only then copied.
    let first_copy = steps
        .iter()
        .position(|step| matches!(step, CopyStep::Copying(_)))
        .unwrap();
    assert_eq!(first_copy, 2);
    assert_eq!(
        steps.last(),
        Some(&CopyStep::Copying(Copied {
            done: 30,
            total: 30
        }))
    );
}

#[test]
fn a_stopped_drop_leaves_nothing_on_the_stick() {
    let from = tempfile::tempdir().unwrap();
    write(&from.path().join("folder/big.bin"), 3 * PIECE);
    let sources = [from.path().join("folder")];
    // Stopped while it is looked at, and while it is copied.
    for stop_when_copying in [false, true] {
        let stick = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let stopped = copy_onto_stoppable(
            stick.path(),
            u64::MAX,
            &[],
            &sources,
            OnConflict::KeepBoth,
            &stop,
            &mut |step| {
                if matches!(step, CopyStep::Copying(_)) == stop_when_copying {
                    stop.store(true, Ordering::Relaxed);
                }
            },
        );
        assert!(matches!(stopped, Err(DriveError::Stopped)), "{stopped:?}");
        let left: Vec<_> = std::fs::read_dir(stick.path()).unwrap().collect();
        assert!(left.is_empty(), "{left:?}");
        // The original is where it was.
        assert!(from.path().join("folder/big.bin").exists());
    }
}
