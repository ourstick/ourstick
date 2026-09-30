//! Building a manifest while encrypting only the files that changed since the
//! last publish. A file whose [`FileStamp`] is as recorded keeps the blob hash
//! recorded with it, without being read (a heuristic; see "Known limits" in
//! `docs/protocol.md`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::build::{hash_file, regular_files, relative_path};
use super::{Entry, FileStamp, Manifest, ManifestError};
use crate::cipher::Cipher;

/// Files encrypted before: path (as in the manifest) → stamp then, blob hash.
pub type KnownFiles = HashMap<String, (FileStamp, [u8; 32])>;

/// The result of [`Manifest::from_dir_reusing`].
#[derive(Debug)]
pub struct Encrypted {
    pub manifest: Manifest,
    /// New blobs, each at `blob_dir/<hex hash>.blob` (a blob shared by
    /// identical files may appear more than once).
    pub new_blobs: Vec<PathBuf>,
    /// Every file with its stamp and blob hash, to record for next time.
    pub files: Vec<(String, FileStamp, [u8; 32])>,
    /// How many files kept their recorded blob without being read.
    pub reused: usize,
}

impl Manifest {
    /// Like [`Manifest::from_dir_keeping_blobs`], but a file whose stamp matches
    /// `known` keeps its recorded blob hash and is neither read nor encrypted.
    /// The caller makes sure those blobs are still available.
    pub fn from_dir_reusing(
        root: impl AsRef<Path>,
        cipher: &dyn Cipher,
        blob_dir: &Path,
        known: &KnownFiles,
    ) -> Result<Encrypted, ManifestError> {
        let root = root.as_ref();
        let mut entries = Vec::new();
        let mut new_blobs = Vec::new();
        let mut files = Vec::new();
        let mut reused = 0;
        for path in regular_files(root)? {
            // Taken before the file is read: if it changes after this, the
            // recorded stamp is older than the file, so it is read next time.
            let metadata = std::fs::metadata(&path).map_err(|e| ManifestError::io(&path, e))?;
            let stamp = FileStamp::of(&metadata);
            let relative = relative_path(root, &path)?;
            let entry = match known.get(&relative) {
                Some((known_stamp, hash)) if *known_stamp == stamp => {
                    reused += 1;
                    Entry {
                        path: relative,
                        size: stamp.size,
                        mtime: mtime_seconds(&metadata),
                        hash: *hash,
                    }
                }
                _ => {
                    let (entry, blob) = hash_file(root, &path, cipher, Some(blob_dir))?;
                    new_blobs.extend(blob);
                    entry
                }
            };
            files.push((entry.path.clone(), stamp, entry.hash));
            entries.push(entry);
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Encrypted {
            manifest: Self {
                entries,
                ..Self::default()
            },
            new_blobs,
            files,
            reused,
        })
    }
}

/// The manifest's mtime: whole seconds, as `hash_file` records it.
fn mtime_seconds(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::blob::BlobHeader;
    use crate::cipher::{ChaChaCipher, CipherError, CipherId};
    use crate::StickKey;

    /// A cipher that counts the blobs it starts, around ChaCha.
    struct Counting {
        inner: ChaChaCipher,
        blobs: AtomicUsize,
    }

    impl Cipher for Counting {
        fn id(&self) -> CipherId {
            self.inner.id()
        }
        fn nonce_prefix_len(&self) -> usize {
            self.inner.nonce_prefix_len()
        }
        fn tag_len(&self) -> usize {
            self.inner.tag_len()
        }
        fn derive_nonce_prefix(&self, plaintext_hash: &blake3::Hash) -> Vec<u8> {
            self.blobs.fetch_add(1, Ordering::SeqCst);
            self.inner.derive_nonce_prefix(plaintext_hash)
        }
        fn seal_chunk(
            &self,
            header: &BlobHeader,
            index: u32,
            is_last: bool,
            plaintext: &[u8],
        ) -> Result<Vec<u8>, CipherError> {
            self.inner.seal_chunk(header, index, is_last, plaintext)
        }
        fn open_chunk(
            &self,
            header: &BlobHeader,
            index: u32,
            is_last: bool,
            ciphertext: &[u8],
        ) -> Result<Vec<u8>, CipherError> {
            self.inner.open_chunk(header, index, is_last, ciphertext)
        }
    }

    fn counting() -> Counting {
        Counting {
            inner: ChaChaCipher::new(&StickKey::from_bytes([1; 32])),
            blobs: AtomicUsize::new(0),
        }
    }

    fn known(encrypted: &Encrypted) -> KnownFiles {
        encrypted
            .files
            .iter()
            .map(|(path, stamp, hash)| (path.clone(), (*stamp, *hash)))
            .collect()
    }

    #[test]
    fn only_changed_and_new_files_are_encrypted() {
        let root = tempfile::tempdir().unwrap();
        let blobs = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a"), b"one").unwrap();
        std::fs::write(root.path().join("b"), b"two").unwrap();
        let cipher = counting();

        let first = Manifest::from_dir_reusing(root.path(), &cipher, blobs.path(), &HashMap::new())
            .unwrap();
        assert_eq!(cipher.blobs.swap(0, Ordering::SeqCst), 2);
        assert_eq!(first.reused, 0);

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.path().join("b"), b"two, changed").unwrap();
        std::fs::write(root.path().join("c"), b"three").unwrap();
        let second =
            Manifest::from_dir_reusing(root.path(), &cipher, blobs.path(), &known(&first)).unwrap();
        assert_eq!(cipher.blobs.swap(0, Ordering::SeqCst), 2, "b and c only");
        assert_eq!(second.reused, 1);
        assert_eq!(second.new_blobs.len(), 2);

        // The result equals a full rebuild.
        let full = Manifest::from_dir(root.path(), &cipher).unwrap();
        assert_eq!(second.manifest, full);
    }

    #[test]
    fn a_matching_stamp_is_trusted_without_reading_the_file() {
        // The known limit, made visible: whatever hash is recorded for an
        // unchanged stamp is used. `share --rescan` passes no known files.
        let root = tempfile::tempdir().unwrap();
        let blobs = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a"), b"one").unwrap();
        let cipher = counting();
        let first = Manifest::from_dir_reusing(root.path(), &cipher, blobs.path(), &HashMap::new())
            .unwrap();
        let mut wrong = known(&first);
        wrong.get_mut("a").unwrap().1 = [9; 32];

        let trusted =
            Manifest::from_dir_reusing(root.path(), &cipher, blobs.path(), &wrong).unwrap();
        assert_eq!(trusted.manifest.entries[0].hash, [9; 32]);
        let rescanned =
            Manifest::from_dir_reusing(root.path(), &cipher, blobs.path(), &HashMap::new())
                .unwrap();
        assert_eq!(rescanned.manifest, first.manifest);
    }
}
