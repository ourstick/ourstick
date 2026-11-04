use super::*;

#[test]
fn a_stick_name_is_made_fit_for_the_manifest() {
    assert_eq!(name_for_manifest("Photos 2024"), "Photos 2024");
    assert_eq!(name_for_manifest("  Για τη Μαρία  "), "Για τη Μαρία");
    // Controls, bidi overrides and zero-width characters go.
    assert_eq!(
        name_for_manifest("tab\there\u{202E}evil\u{200B}x"),
        "tabhereevilx"
    );
    // Cut by scalar values, not bytes.
    let long = "Ω".repeat(70);
    let cut = name_for_manifest(&long);
    assert_eq!(cut.chars().count(), MAX_NAME_CHARS);
    for name in ["\u{7}", "x".repeat(100).as_str(), "a\u{2028}b", ""] {
        validate_name(&name_for_manifest(name)).unwrap();
    }
}
use crate::manifest::Entry;
use rand::{Rng, SeedableRng};

pub(super) fn entry(path: &str) -> Entry {
    Entry {
        path: path.to_owned(),
        size: 5,
        mtime: 1_700_000_000,
        hash: [9; 32],
    }
}

pub(super) fn manifest(paths: &[&str]) -> Manifest {
    Manifest {
        entries: paths.iter().map(|p| entry(p)).collect(),
        ..Manifest::default()
    }
}

/// Encodes without validation, to build hostile input for the decoder.
pub(super) fn encode_unchecked(manifest: &Manifest) -> Vec<u8> {
    let mut bytes = vec![MANIFEST_FORMAT_VERSION];
    bytes.extend(postcard::to_allocvec(manifest).unwrap());
    bytes
}

#[test]
fn roundtrip_preserves_manifest() {
    let original = manifest(&["a.txt", "photos/2024/cat.jpg", "z"]);
    let decoded = Manifest::from_bytes(&original.to_bytes().unwrap()).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn roundtrip_of_manifest_from_dir() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/one.txt"), b"one").unwrap();
    std::fs::write(dir.path().join("two.txt"), b"two").unwrap();

    let original = Manifest::from_dir(dir.path(), &crate::cipher::NoCipher).unwrap();
    let decoded = Manifest::from_bytes(&original.to_bytes().unwrap()).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn roundtrip_of_empty_manifest() {
    let bytes = Manifest::empty().to_bytes().unwrap();
    // partial = false, name = "", no entries.
    assert_eq!(bytes, [MANIFEST_FORMAT_VERSION, 0, 0, 0]);
    assert_eq!(Manifest::from_bytes(&bytes).unwrap(), Manifest::empty());
}

#[test]
fn encoding_has_exact_field_order() {
    let manifest = Manifest {
        partial: true,
        name: "Ab".to_owned(),
        entries: vec![Entry {
            path: "a".to_owned(),
            size: 1,
            mtime: 2,
            hash: [3; 32],
        }],
    };
    let mut expected = vec![
        MANIFEST_FORMAT_VERSION,
        1,    // partial
        2,    // name length
        b'A', // name
        b'b',
        1,    // entry count
        1,    // path length
        b'a', // path
        1,    // size
        2,    // mtime
    ];
    expected.extend([3; 32]); // hash, raw
    assert_eq!(manifest.to_bytes().unwrap(), expected);
}

#[test]
fn encoding_is_deterministic() {
    let manifest = manifest(&["a", "b/c"]);
    assert_eq!(manifest.to_bytes().unwrap(), manifest.to_bytes().unwrap());
}

#[test]
fn to_bytes_rejects_invalid_manifest() {
    assert!(matches!(
        manifest(&["../escape"]).to_bytes(),
        Err(ManifestError::InvalidPath(_))
    ));
    assert!(matches!(
        manifest(&["b", "a"]).to_bytes(),
        Err(ManifestError::NotSorted(_))
    ));
}

#[test]
fn from_bytes_rejects_empty_input() {
    assert!(matches!(
        Manifest::from_bytes(&[]),
        Err(ManifestError::EmptyInput)
    ));
}

#[test]
fn from_bytes_rejects_unknown_version() {
    let mut bytes = manifest(&["a"]).to_bytes().unwrap();
    bytes[0] = MANIFEST_FORMAT_VERSION + 1;
    assert!(matches!(
        Manifest::from_bytes(&bytes),
        Err(ManifestError::UnsupportedVersion(v)) if v == MANIFEST_FORMAT_VERSION + 1
    ));
}

#[test]
fn from_bytes_rejects_trailing_bytes() {
    let mut bytes = manifest(&["a"]).to_bytes().unwrap();
    bytes.extend([0, 0]);
    assert!(matches!(
        Manifest::from_bytes(&bytes),
        Err(ManifestError::TrailingBytes(2))
    ));
}

#[test]
fn from_bytes_rejects_unsafe_paths() {
    let unsafe_paths = [
        "",
        "/etc/passwd",
        "../escape",
        "a/../../escape",
        "a/./b",
        "a//b",
        "a/",
        ".",
        "..\\escape",
        "a\\b",
        "a\0b",
    ];
    for path in unsafe_paths {
        let bytes = encode_unchecked(&manifest(&[path]));
        assert!(
            matches!(
                Manifest::from_bytes(&bytes),
                Err(ManifestError::InvalidPath(_))
            ),
            "accepted unsafe path {path:?}"
        );
    }
}

#[test]
fn from_bytes_accepts_unusual_but_safe_paths() {
    for path in [
        "...",
        "..a",
        "a..",
        ".hidden",
        "with space/ünïcödé.txt",
        "a:b",
    ] {
        let bytes = encode_unchecked(&manifest(&[path]));
        assert!(Manifest::from_bytes(&bytes).is_ok(), "rejected {path:?}");
    }
}

#[test]
fn from_bytes_rejects_unsorted_and_duplicate_entries() {
    for paths in [&["b", "a"][..], &["a", "a"][..]] {
        let bytes = encode_unchecked(&manifest(paths));
        assert!(matches!(
            Manifest::from_bytes(&bytes),
            Err(ManifestError::NotSorted(_))
        ));
    }
}

#[test]
fn from_bytes_rejects_every_truncation() {
    let bytes = manifest(&["a.txt", "b/c.txt"]).to_bytes().unwrap();
    for length in 0..bytes.len() {
        assert!(
            Manifest::from_bytes(&bytes[..length]).is_err(),
            "accepted truncation to {length} bytes"
        );
    }
}

#[test]
fn from_bytes_rejects_huge_length_prefix_without_allocating() {
    // Claims u32::MAX entries but carries none.
    let bytes = [MANIFEST_FORMAT_VERSION, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
    assert!(Manifest::from_bytes(&bytes).is_err());
}

#[test]
fn from_bytes_never_panics_on_random_input() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x0005_71C4);
    for _ in 0..10_000 {
        let length = rng.gen_range(0..256);
        let mut bytes: Vec<u8> = (0..length).map(|_| rng.gen()).collect();
        if let Some(first) = bytes.first_mut() {
            *first = MANIFEST_FORMAT_VERSION;
        }
        let _ = Manifest::from_bytes(&bytes);
    }
}

#[test]
fn from_bytes_never_panics_on_mutated_valid_input() {
    let valid = manifest(&["a.txt", "b/c.txt", "d"]).to_bytes().unwrap();
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x0005_71C5);
    for _ in 0..10_000 {
        let mut bytes = valid.clone();
        let index = rng.gen_range(1..bytes.len());
        bytes[index] = rng.gen();
        let _ = Manifest::from_bytes(&bytes);
    }
}

#[test]
fn version_1_is_still_read_as_complete_and_nameless() {
    let old = manifest(&["a.txt", "b.txt"]);
    let mut bytes = vec![1];
    bytes.extend(postcard::to_allocvec(&old.entries).unwrap());
    let read = Manifest::from_bytes(&bytes).unwrap();
    assert_eq!(read.entries, old.entries);
    assert!(!read.partial);
    assert_eq!(read.name, "");
}

#[test]
fn names_follow_the_wire_rule() {
    for good in [
        "",
        "For Maria",
        "Φωτογραφίες 2024",
        "😀 photos",
        &"x".repeat(60),
    ] {
        assert!(validate_name(good).is_ok(), "{good:?}");
    }
    for bad in [
        "tab\there",
        "line\nbreak",
        "\u{202E}evil",
        "zero\u{200B}width",
        "\u{2028}",
        "\u{FEFF}",
        &"x".repeat(61),
    ] {
        assert!(
            matches!(validate_name(bad), Err(ManifestError::InvalidName(_))),
            "{bad:?}"
        );
        let named = Manifest {
            name: bad.to_owned(),
            ..Manifest::default()
        };
        assert!(named.to_bytes().is_err(), "an owner never sends {bad:?}");
        assert!(
            Manifest::from_bytes(&encode_unchecked(&named)).is_err(),
            "a reader refuses {bad:?}"
        );
    }
}
