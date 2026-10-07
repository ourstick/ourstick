//! The readers of a task over several disks, and what stops them.

use std::io::Read;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::*;

fn disks() -> Arc<DiskTurns> {
    Arc::new(DiskTurns::new(Arc::new(SystemTimer::default())))
}

/// Reads a few bytes of a small file on `id` through `readers`, leaving
/// its window (and so the disk's turn) held.
fn hold(readers: &mut Readers, id: DiskId) {
    let reader = readers.on(id);
    let mut timed = reader.open(std::io::Cursor::new(vec![0u8; 10]), 10);
    timed.read_exact(&mut [0; 5]).unwrap();
}

fn held(disks: &DiskTurns, id: DiskId) -> bool {
    lock(&disks.get(id).state).held
}

#[test]
fn a_task_holds_one_disks_turn_at_a_time() {
    let disks = disks();
    let mut readers = Readers::new(disks.clone(), false, || Ok(()));
    let a = DiskId::Device(1);
    let b = DiskId::Device(2);
    hold(&mut readers, a.clone());
    assert!(held(&disks, a.clone()));
    // Reading another disk gives the first one's turn back first.
    hold(&mut readers, b.clone());
    assert!(!held(&disks, a.clone()) && held(&disks, b.clone()));
    // Back to the first: the second's goes.
    hold(&mut readers, a.clone());
    assert!(held(&disks, a.clone()) && !held(&disks, b.clone()));
    // The end of a part gives every turn back.
    readers.end_windows();
    assert!(!held(&disks, a.clone()) && !held(&disks, b));
    // None gives way on an idle disk.
    assert!(!readers.giving_way());
}

#[test]
fn readers_say_whether_any_of_their_disks_gives_way() {
    let disks = disks();
    let mut readers = Readers::new(disks.clone(), true, || Ok(()));
    hold(&mut readers, DiskId::Device(7));
    readers.end_windows();
    assert!(!readers.giving_way());
    // Its disk judged busy (as the turns' tests make it), it says so.
    {
        let disk = disks.get(DiskId::Device(7));
        let mut state = lock(&disk.state);
        for took in [100, 100, 100, 900, 900] {
            state.load.after_window(
                crate::disk::share::ReaderKind::Folder,
                2,
                64 << 20,
                Duration::from_millis(took),
                None,
            );
        }
    }
    assert!(readers.giving_way());
    readers.set_urgent(false);
}

#[test]
fn a_stop_on_drop_sets_its_flag_when_dropped() {
    let stop = StopOnDrop::default();
    let flag = stop.flag();
    assert!(!flag.load(Ordering::Relaxed));
    drop(stop);
    assert!(flag.load(Ordering::Relaxed));
}

#[test]
fn a_files_disk_is_its_folders() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("a.txt");
    std::fs::write(&file, b"a").unwrap();
    let metadata = std::fs::symlink_metadata(&file).unwrap();
    assert_eq!(DiskId::of_file(&file, &metadata), DiskId::of(folder.path()));
}
