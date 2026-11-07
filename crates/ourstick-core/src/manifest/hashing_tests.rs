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
        3 * 1024 * 1024 + 17,
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

/// Outboards written as the catalogue writes them, against ones made by code
/// of their own (BLAKE3 from its definition, checked on BLAKE3's test
/// vectors) for `size` bytes of `i % 251`. Outboards that an older version
/// wrote must stay valid when bao-tree changes (ADR 0017 rule 1): else every
/// file is read again at its first transfer.
#[test]
fn outboards_keep_their_format() {
    let golden: [(usize, &str, &[&str]); 4] = [
        (
            16384,
            "f875d6646de28985646f34ee13be9a576fd515f76b5b0a26bb324735041ddde4",
            &[],
        ),
        (
            16385,
            "1dabe216be2578830263b049de1639f39f05a4da616b9b78c7a5e4e41662fd1f",
            &[
                "5384f9b342c6cb86badedcbe662b0e980476a8db17db3cc1f13b19329749ab1f",
                "7eec20c857faf93c8102396e3b2ea90bcdf693ebd5cbd77211e5f606d72bc1a5",
            ],
        ),
        (
            40000,
            "ccd32b4544a24c50fbefb249e10a8fcedc26e2794c79eb8a44ad0631bf07f53f",
            &[
                "1e03bf3657c5076623059f21d92f2ec634dd276b07e3760625c73416f343d857",
                "083663d9bddc14d40cc2f3869e01c5bebde7632dfea165eda38af01d93d5045b",
                "5384f9b342c6cb86badedcbe662b0e980476a8db17db3cc1f13b19329749ab1f",
                "406632eceb31aa6735f072ad0554f5963f4082b5139978455b44f67732cf949d",
            ],
        ),
        (
            81820,
            "c39931dacfab86e74e0f02f7048b8786626e0cc27ed191c3d155449fb393438c",
            &[
                "88da791e722ac59740d9a44402e384ad99f1c79d4bab7449a0c2da1fafb91fb7",
                "cc366e81885ce0b7bfa39ced9877f3027d1723d9c56e891524fb92705ba7a473",
                "1e03bf3657c5076623059f21d92f2ec634dd276b07e3760625c73416f343d857",
                "95d13beb8a252ab0a9ad34b90063717be147ef23b67b90a59d3d8ce3cc5e076a",
                "5384f9b342c6cb86badedcbe662b0e980476a8db17db3cc1f13b19329749ab1f",
                "406632eceb31aa6735f072ad0554f5963f4082b5139978455b44f67732cf949d",
                "bccbba6c1168012dc733c21a85cc8500576e28489a119e848307770260b3c87f",
                "762831a5a6c2ca9fad845da11b211517d966e798daa3de1a9cb06e2e61cbf785",
            ],
        ),
    ];
    for (size, root, outboard) in golden {
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let tree = BaoTree::new(size as u64, BLOCK_SIZE);
        let mut written = PreOrderOutboard {
            root: blake3::Hash::from_bytes([0; 32]),
            tree,
            data: Vec::new(),
        };
        let made = bao_tree::io::sync::outboard(&mut Cursor::new(&data), tree, &mut written);
        assert_eq!(made.unwrap().to_hex().as_str(), root, "size {size}");
        let hex: String = written.data.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, outboard.concat(), "size {size}");
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

#[test]
fn without_a_folder_nothing_is_written() {
    let data = plaintext(200 * 1024);
    let mut blob = Vec::new();
    encrypt_blob(&cipher(), &mut Cursor::new(&data), &mut blob).unwrap();
    let hashes = hash_plaintext(&cipher(), &mut Cursor::new(&data), None).unwrap();
    assert_eq!(hashes.blob_hash, *blake3::hash(&blob).as_bytes());
}

#[test]
fn identical_files_share_one_outboard() {
    let dir = tempfile::tempdir().unwrap();
    let data = plaintext(100 * 1024);
    for _ in 0..2 {
        hash_plaintext(&cipher(), &mut Cursor::new(&data), Some(dir.path())).unwrap();
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn an_outboard_cut_short_or_damaged_is_made_again() {
    let dir = tempfile::tempdir().unwrap();
    let data = plaintext(100 * 1024);
    let hashes = hash_plaintext(&cipher(), &mut Cursor::new(&data), Some(dir.path())).unwrap();
    let path = outboard_path(dir.path(), &hashes.blob_hash);
    let whole = std::fs::read(&path).unwrap();
    // A crash left it short; then a bit changed.
    std::fs::write(&path, &whole[..10]).unwrap();
    hash_plaintext(&cipher(), &mut Cursor::new(&data), Some(dir.path())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), whole);
    let mut damaged = whole.clone();
    damaged[5] ^= 1;
    std::fs::write(&path, &damaged).unwrap();
    hash_plaintext(&cipher(), &mut Cursor::new(&data), Some(dir.path())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), whole);
}

/// A plaintext whose second read fails.
struct Failing {
    data: Cursor<Vec<u8>>,
    seeked: bool,
}

impl Read for Failing {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.seeked && self.data.position() > 70 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "unplugged",
            ));
        }
        self.data.read(out)
    }
}

impl Seek for Failing {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.seeked = true;
        self.data.seek(to)
    }
}

#[test]
fn a_read_that_fails_is_an_error_not_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut failing = Failing {
        data: Cursor::new(plaintext(200 * 1024)),
        seeked: false,
    };
    let result = hash_plaintext(&cipher(), &mut failing, Some(dir.path()));
    assert!(
        matches!(result, Err(CipherError::Io(_))),
        "{:?}",
        result.err()
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn without_a_cipher_the_blob_is_hashed_the_same_way() {
    let dir = tempfile::tempdir().unwrap();
    let data = plaintext(100 * 1024);
    let mut blob = Vec::new();
    encrypt_blob(&crate::cipher::NoCipher, &mut Cursor::new(&data), &mut blob).unwrap();
    let hashes = hash_plaintext(
        &crate::cipher::NoCipher,
        &mut Cursor::new(&data),
        Some(dir.path()),
    )
    .unwrap();
    assert_eq!(hashes.blob_hash, *blake3::hash(&blob).as_bytes());
    let expected = PreOrderMemOutboard::create(&blob, BLOCK_SIZE);
    assert_eq!(
        std::fs::read(outboard_path(dir.path(), &hashes.blob_hash)).unwrap(),
        expected.data
    );
}
