use super::*;
use crate::manifest::Entry;

fn manifest(entries: &[(&str, u8)]) -> Manifest {
    Manifest {
        entries: entries
            .iter()
            .map(|(path, hash)| Entry {
                path: (*path).to_owned(),
                size: 1,
                mtime: 0,
                hash: [*hash; 32],
            })
            .collect(),
        ..Manifest::default()
    }
}

#[test]
fn summary_lists_added_changed_and_removed_paths() {
    let old = manifest(&[("a", 1), ("b", 2), ("c", 3)]);
    let new = manifest(&[("a", 1), ("b", 9), ("d", 4)]);
    assert_eq!(
        ChangeSummary::between(&old, &new),
        ChangeSummary {
            added: vec!["d".to_owned()],
            changed: vec!["b".to_owned()],
            removed: vec!["c".to_owned()],
        }
    );
    assert_eq!(ChangeSummary::between(&new, &new), ChangeSummary::default());
}

#[tokio::test]
async fn a_file_being_written_is_not_settled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("growing.bin");
    std::fs::write(&path, b"").unwrap();
    assert!(is_settled(dir.path(), Duration::from_millis(50)).await);

    let writer = std::thread::spawn(move || {
        use std::io::Write;
        let mut file = std::fs::File::options().append(true).open(path).unwrap();
        for _ in 0..20 {
            file.write_all(&[0; 1024]).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    assert!(!is_settled(dir.path(), Duration::from_millis(100)).await);
    writer.join().unwrap();
    assert!(is_settled(dir.path(), Duration::from_millis(50)).await);
}

#[test]
fn looks_with_nothing_new_come_ever_more_slowly() {
    let retry = Duration::from_secs(30);
    let waits: Vec<u64> = (0..8)
        .map(|looks| look_again_after(retry, looks).as_secs())
        .collect();
    assert_eq!(waits, [30, 60, 120, 240, 480, 960, 1800, 1800]);
    assert_eq!(look_again_after(retry, u32::MAX), LONGEST_LOOK_AGAIN);
    // A retry longer than the longest wait is kept as it is.
    let long = Duration::from_secs(3600);
    assert_eq!(look_again_after(long, 3), long);
}
