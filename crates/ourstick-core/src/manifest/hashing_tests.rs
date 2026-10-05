use std::io::Cursor;

use bao_tree::io::outboard::PreOrderMemOutboard;

use super::*;
use crate::blob::encrypt_blob;
use crate::cipher::{ChaChaCipher, StickKey};

fn cipher() -> ChaChaCipher {
    ChaChaCipher::new(&StickKey::from_bytes([4; 32]))
}

fn plaintext(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i * 31 + 7) as u8).collect()
}

#[test]
fn the_hash_and_outboard_are_those_of_the_blob() {
    let dir = tempfile::tempdir().unwrap();
    for size in [
        0,
        1,
        16 * 1024 - 40,
        16 * 1024 + 1,
        64 * 1024,
        300 * 1024 + 3,
    ] {
        let data = plaintext(size);
        let mut blob = Vec::new();
        encrypt_blob(&cipher(), &mut Cursor::new(&data), &mut blob).unwrap();
        let hashes = hash_plaintext(&cipher(), &mut Cursor::new(&data), Some(dir.path())).unwrap();
        assert_eq!(
            hashes.blob_hash,
            *blake3::hash(&blob).as_bytes(),
            "size {size}"
        );
        assert_eq!(hashes.plaintext_len, size as u64);
        assert_eq!(hashes.plaintext_hash, *blake3::hash(&data).as_bytes());
        // The outboard iroh-blobs would make for this blob.
        let expected = PreOrderMemOutboard::create(&blob, BLOCK_SIZE);
        assert_eq!(expected.root.as_bytes(), &hashes.blob_hash);
        let kept = std::fs::read(outboard_path(dir.path(), &hashes.blob_hash));
        if expected.data.is_empty() {
            assert!(kept.is_err(), "size {size}: one group, no outboard");
        } else {
            assert_eq!(kept.unwrap(), expected.data, "size {size}");
        }
    }
}

/// A plaintext that reads one way, then another after a seek.
struct Changing {
    first: Cursor<Vec<u8>>,
    second: Cursor<Vec<u8>>,
    seeked: bool,
}

impl Read for Changing {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.seeked {
            self.second.read(out)
        } else {
            self.first.read(out)
        }
    }
}

impl Seek for Changing {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        if matches!(to, SeekFrom::Start(_)) && self.first.position() > 0 {
            self.seeked = true;
        }
        self.second.seek(to)
    }
}

#[test]
fn a_file_that_changes_between_the_reads_keeps_no_outboard() {
    let original = plaintext(200 * 1024);
    let mut longer = original.clone();
    longer.push(1);
    let mut altered = original.clone();
    altered[100 * 1024] ^= 1;
    let shorter = original[..original.len() - 1].to_vec();
    for second in [longer, altered, shorter] {
        let dir = tempfile::tempdir().unwrap();
        let mut reading = Changing {
            first: Cursor::new(original.clone()),
            second: Cursor::new(second),
            seeked: false,
        };
        let result = hash_plaintext(&cipher(), &mut reading, Some(dir.path()));
        assert!(
            matches!(result, Err(CipherError::PlaintextChanged)),
            "{:?}",
            result.map(|hashes| hashes.plaintext_len)
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[test]
fn only_the_outboards_of_the_version_stay() {
    let dir = tempfile::tempdir().unwrap();
    let kept = [1u8; 32];
    let gone = [2u8; 32];
    std::fs::write(outboard_path(dir.path(), &kept), b"k").unwrap();
    std::fs::write(outboard_path(dir.path(), &gone), b"g").unwrap();
    std::fs::write(dir.path().join("x.obao.partial"), b"p").unwrap();
    keep_only_outboards(dir.path(), &[kept].into_iter().collect()).unwrap();
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(left, vec![outboard_path(dir.path(), &kept)]);
    // A stick that never had one: nothing to do.
    keep_only_outboards(&dir.path().join("none"), &Default::default()).unwrap();
}
