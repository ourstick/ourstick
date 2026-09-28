//! Wire format of a manifest: one format-version byte, then the postcard encoding.
//!
//! Decoding treats its input as untrusted: it never panics, and it rejects any
//! manifest a reader could not safely write to disk. See `docs/protocol.md`.

use super::{Manifest, ManifestError};

/// Version of the manifest wire format written by [`Manifest::to_bytes`].
pub const MANIFEST_FORMAT_VERSION: u8 = 1;

impl Manifest {
    /// Encodes the manifest into its wire format.
    ///
    /// Fails if the manifest is not valid (see [`Manifest`]), so a peer never
    /// sends a manifest that other peers would reject.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ManifestError> {
        self.validate()?;
        let mut bytes = vec![MANIFEST_FORMAT_VERSION];
        bytes.extend(postcard::to_allocvec(self)?);
        Ok(bytes)
    }

    /// Decodes a manifest from its wire format and validates it.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ManifestError> {
        let (&version, body) = bytes.split_first().ok_or(ManifestError::EmptyInput)?;
        if version != MANIFEST_FORMAT_VERSION {
            return Err(ManifestError::UnsupportedVersion(version));
        }
        let (manifest, rest): (Self, _) = postcard::take_from_bytes(body)?;
        if !rest.is_empty() {
            return Err(ManifestError::TrailingBytes(rest.len()));
        }
        manifest.validate()?;
        Ok(manifest)
    }

    /// Checks that every path is safe and that entries are strictly sorted by path.
    fn validate(&self) -> Result<(), ManifestError> {
        for entry in &self.entries {
            validate_path(&entry.path)?;
        }
        for pair in self.entries.windows(2) {
            if pair[0].path >= pair[1].path {
                return Err(ManifestError::NotSorted(pair[1].path.clone()));
            }
        }
        Ok(())
    }
}

/// Checks that `path` is a relative, `/`-separated path that stays inside the
/// stick root on every platform.
pub(super) fn validate_path(path: &str) -> Result<(), ManifestError> {
    let is_valid = !path.is_empty()
        && !path.contains(['\\', '\0'])
        && path
            .split('/')
            .all(|component| !matches!(component, "" | "." | ".."));
    if is_valid {
        Ok(())
    } else {
        Err(ManifestError::InvalidPath(path.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Entry;
    use rand::{Rng, SeedableRng};

    fn entry(path: &str) -> Entry {
        Entry {
            path: path.to_owned(),
            size: 5,
            mtime: 1_700_000_000,
            hash: [9; 32],
        }
    }

    fn manifest(paths: &[&str]) -> Manifest {
        Manifest {
            entries: paths.iter().map(|p| entry(p)).collect(),
        }
    }

    /// Encodes without validation, to build hostile input for the decoder.
    fn encode_unchecked(manifest: &Manifest) -> Vec<u8> {
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
        assert_eq!(bytes, [MANIFEST_FORMAT_VERSION, 0]);
        assert_eq!(Manifest::from_bytes(&bytes).unwrap(), Manifest::empty());
    }

    #[test]
    fn encoding_has_exact_field_order() {
        let manifest = Manifest {
            entries: vec![Entry {
                path: "a".to_owned(),
                size: 1,
                mtime: 2,
                hash: [3; 32],
            }],
        };
        let mut expected = vec![
            MANIFEST_FORMAT_VERSION,
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
}
