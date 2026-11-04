//! The bounds of a manifest: what a reader takes on, by size.

use super::tests::{encode_unchecked, entry, manifest};
use super::*;
use crate::manifest::{MAX_ENTRIES, MAX_PATH_BYTES, MAX_PATH_DEPTH, MAX_TOTAL_BYTES};

#[test]
fn a_path_too_long_or_too_deep_is_refused_and_not_repeated_whole() {
    let deepest = vec!["a"; MAX_PATH_DEPTH].join("/");
    let longest = format!("{}/b", "x".repeat(MAX_PATH_BYTES - 2));
    for path in [&deepest, &longest] {
        let bytes = encode_unchecked(&manifest(&[path]));
        assert!(Manifest::from_bytes(&bytes).is_ok(), "{} bytes", path.len());
    }
    let too_deep = vec!["a"; MAX_PATH_DEPTH + 1].join("/");
    let too_long = format!("{}/b", "x".repeat(MAX_PATH_BYTES - 1));
    // The attack: a path of a million bytes.
    let hostile = vec!["a"; 500_000].join("/");
    for path in [&too_deep, &too_long, &hostile] {
        let bytes = encode_unchecked(&manifest(&[path]));
        match Manifest::from_bytes(&bytes) {
            Err(error @ ManifestError::InvalidPath(_)) => {
                assert!(error.to_string().len() < 300, "the whole path in the error")
            }
            other => panic!("accepted {} bytes: {other:?}", path.len()),
        }
    }
}

#[test]
fn sizes_adding_up_past_any_disk_are_refused() {
    let sized = |sizes: &[u64]| Manifest {
        entries: sizes
            .iter()
            .enumerate()
            .map(|(index, &size)| Entry {
                size,
                ..entry(&format!("f{index}"))
            })
            .collect(),
        ..Manifest::default()
    };
    // Each would pass alone; together they wrap a u64 to 1 KB.
    let wrapping = sized(&[1 << 63, 1 << 63, 1024]);
    for manifest in [wrapping, sized(&[u64::MAX]), sized(&[MAX_TOTAL_BYTES, 1])] {
        assert!(matches!(
            Manifest::from_bytes(&encode_unchecked(&manifest)),
            Err(ManifestError::TooManyBytes)
        ));
    }
    let fine = sized(&[MAX_TOTAL_BYTES - 1, 1]);
    assert_eq!(
        Manifest::from_bytes(&encode_unchecked(&fine))
            .unwrap()
            .total_size(),
        MAX_TOTAL_BYTES
    );
}

#[test]
fn more_files_than_a_stick_may_have_are_refused() {
    let names: Vec<String> = (0..=MAX_ENTRIES)
        .map(|index| format!("{index:08}"))
        .collect();
    let mut many = Manifest {
        entries: names.iter().map(|name| entry(name)).collect(),
        ..Manifest::default()
    };
    match Manifest::from_bytes(&encode_unchecked(&many)) {
        Err(ManifestError::TooManyFiles { count }) => assert_eq!(count, MAX_ENTRIES + 1),
        other => panic!("{:?}", other.map(|manifest| manifest.entries.len())),
    }
    assert!(matches!(
        many.to_bytes(),
        Err(ManifestError::TooManyFiles { .. })
    ));
    many.entries.pop();
    assert_eq!(
        Manifest::from_bytes(&many.to_bytes().unwrap())
            .unwrap()
            .entries
            .len(),
        MAX_ENTRIES
    );
}

#[test]
fn folders_are_counted_once_and_too_many_are_refused() {
    let paths = |list: &[&str]| -> Vec<Entry> { list.iter().map(|path| entry(path)).collect() };
    // "a", "b" and "b/x": 1 + 1 + 3 bytes.
    assert_eq!(
        folder_bytes(&paths(&["a/b", "a/c", "a-file", "b/x/y", "b/x/z"])),
        5
    );
    assert_eq!(folder_bytes(&paths(&["a", "a/b"])), 1);
    // Each path within the limits, each its own deep tree of long names.
    let deep = |first: usize| {
        let mut path = format!("{first:06}");
        while path.len() + 41 < MAX_PATH_BYTES {
            path.push('/');
            path.push_str(&"n".repeat(40));
        }
        path + "/f"
    };
    let hostile = Manifest {
        entries: (0..200).map(|first| entry(&deep(first))).collect(),
        ..Manifest::default()
    };
    assert!(matches!(
        Manifest::from_bytes(&encode_unchecked(&hostile)),
        Err(ManifestError::TooManyFolders)
    ));
    let few = Manifest {
        entries: (0..20).map(|first| entry(&deep(first))).collect(),
        ..Manifest::default()
    };
    assert!(Manifest::from_bytes(&encode_unchecked(&few)).is_ok());
}
