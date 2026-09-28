use std::io::Cursor;

use rand::{Rng, SeedableRng};

use super::*;
use crate::cipher::{ChaChaCipher, NoCipher, StickKey};

const TAG: usize = 16;
const PREFIX: usize = 19;
const CHACHA_HEADER: usize = 2 + PREFIX;

fn chacha() -> ChaChaCipher {
    ChaChaCipher::new(&StickKey::from_bytes([3; 32]))
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn encrypt(cipher: &dyn Cipher, plaintext: &[u8]) -> Vec<u8> {
    let mut blob = Vec::new();
    encrypt_blob(cipher, &mut Cursor::new(plaintext), &mut blob).unwrap();
    blob
}

fn decrypt(cipher: &dyn Cipher, blob: &[u8]) -> Result<Vec<u8>, CipherError> {
    let mut plaintext = Vec::new();
    decrypt_blob(cipher, &mut Cursor::new(blob), &mut plaintext)?;
    Ok(plaintext)
}

const SIZES: [usize; 7] = [
    0,
    1,
    CHUNK_SIZE - 1,
    CHUNK_SIZE,
    CHUNK_SIZE + 1,
    2 * CHUNK_SIZE,
    3 * CHUNK_SIZE + 5,
];

#[test]
fn header_bytes_layout() {
    let header = BlobHeader::new(CipherId::XChaCha20Poly1305, vec![9; PREFIX]);
    let bytes = header.to_bytes();
    assert_eq!(bytes.len(), CHACHA_HEADER);
    assert_eq!(bytes[0], BLOB_FORMAT_VERSION);
    assert_eq!(bytes[1], 1);
    assert_eq!(&bytes[2..], &[9; PREFIX]);
    assert_eq!(header.version(), BLOB_FORMAT_VERSION);
    assert_eq!(header.cipher(), CipherId::XChaCha20Poly1305);
    assert_eq!(header.nonce_prefix(), &[9; PREFIX]);
}

#[test]
fn header_read_from_roundtrips() {
    let header = BlobHeader::new(CipherId::XChaCha20Poly1305, vec![4; PREFIX]);
    let read = BlobHeader::read_from(&mut Cursor::new(header.to_bytes()), &chacha()).unwrap();
    assert_eq!(read, header);
}

#[test]
fn header_read_from_rejects_bad_headers() {
    let read = |bytes: &[u8]| BlobHeader::read_from(&mut Cursor::new(bytes), &chacha());

    assert!(matches!(read(&[]), Err(CipherError::Truncated)));
    assert!(matches!(read(&[1]), Err(CipherError::Truncated)));
    assert!(matches!(read(&[1, 1, 0, 0]), Err(CipherError::Truncated)));
    assert!(matches!(
        read(&[2, 1]),
        Err(CipherError::UnsupportedVersion(2))
    ));
    assert!(matches!(read(&[1, 9]), Err(CipherError::UnknownCipher(9))));
    assert!(matches!(
        read(&[1, 0]),
        Err(CipherError::CipherMismatch {
            expected: CipherId::XChaCha20Poly1305,
            found: CipherId::None
        })
    ));
}

#[test]
fn roundtrip_all_sizes_with_both_ciphers() {
    let ciphers: [&dyn Cipher; 2] = [&NoCipher, &chacha()];
    for cipher in ciphers {
        for size in SIZES {
            let plaintext = pattern(size);
            let blob = encrypt(cipher, &plaintext);
            assert_eq!(decrypt(cipher, &blob).unwrap(), plaintext, "size {size}");
        }
    }
}

#[test]
fn chacha_blob_length_and_info() {
    for size in SIZES {
        let chunk_count = size.div_ceil(CHUNK_SIZE).max(1);
        let expected = CHACHA_HEADER + size + chunk_count * TAG;

        let mut blob = Vec::new();
        let info = encrypt_blob(&chacha(), &mut Cursor::new(pattern(size)), &mut blob).unwrap();
        assert_eq!(blob.len(), expected, "size {size}");
        assert_eq!(
            info,
            BlobInfo {
                plaintext_len: size as u64,
                blob_len: expected as u64
            }
        );
    }
}

#[test]
fn no_cipher_blob_is_header_then_plaintext() {
    let blob = encrypt(&NoCipher, b"hello");
    assert_eq!(blob, [&[BLOB_FORMAT_VERSION, 0][..], b"hello"].concat());
}

#[test]
fn chacha_blob_hides_plaintext() {
    let plaintext = vec![0u8; 1000];
    let blob = encrypt(&chacha(), &plaintext);
    assert!(!blob.windows(64).any(|window| window == &plaintext[..64]));
}

#[test]
fn encryption_is_deterministic_per_key_and_plaintext() {
    let plaintext = pattern(CHUNK_SIZE + 10);
    assert_eq!(
        encrypt(&chacha(), &plaintext),
        encrypt(&chacha(), &plaintext)
    );

    let other_key = ChaChaCipher::new(&StickKey::from_bytes([4; 32]));
    assert_ne!(
        encrypt(&chacha(), &plaintext),
        encrypt(&other_key, &plaintext)
    );

    let mut changed = plaintext.clone();
    changed[0] ^= 1;
    let (first, second) = (encrypt(&chacha(), &plaintext), encrypt(&chacha(), &changed));
    assert_ne!(first[..CHACHA_HEADER], second[..CHACHA_HEADER]);
}

#[test]
fn encrypt_starts_at_current_position() {
    let mut input = Cursor::new(b"skipped-kept".to_vec());
    input.set_position(8);
    let mut blob = Vec::new();
    encrypt_blob(&chacha(), &mut input, &mut blob).unwrap();
    assert_eq!(decrypt(&chacha(), &blob).unwrap(), b"kept");
    assert_eq!(blob, encrypt(&chacha(), b"kept"));
}

/// A reader whose content changes after the first seek, like a file being
/// written to while it is encrypted.
struct ChangesAfterSeek {
    inner: Cursor<Vec<u8>>,
}

impl Read for ChangesAfterSeek {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Seek for ChangesAfterSeek {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.get_mut()[0] ^= 1;
        self.inner.seek(position)
    }
}

#[test]
fn encrypt_detects_plaintext_changed_between_passes() {
    let mut input = ChangesAfterSeek {
        inner: Cursor::new(pattern(100)),
    };
    let result = encrypt_blob(&chacha(), &mut input, &mut Vec::new());
    assert!(matches!(result, Err(CipherError::PlaintextChanged)));
}

#[test]
fn decrypt_rejects_wrong_key() {
    let blob = encrypt(&chacha(), b"secret");
    let other_key = ChaChaCipher::new(&StickKey::from_bytes([4; 32]));
    assert!(matches!(
        decrypt(&other_key, &blob),
        Err(CipherError::Authentication(0))
    ));
}

#[test]
fn decrypt_rejects_header_only_chacha_blob() {
    let blob = encrypt(&chacha(), b"");
    assert!(matches!(
        decrypt(&chacha(), &blob[..CHACHA_HEADER]),
        Err(CipherError::Truncated)
    ));
}

#[test]
fn decrypt_rejects_truncation_at_chunk_boundary() {
    let blob = encrypt(&chacha(), &pattern(2 * CHUNK_SIZE + 1));
    let first_two_chunks = CHACHA_HEADER + 2 * (CHUNK_SIZE + TAG);
    assert!(matches!(
        decrypt(&chacha(), &blob[..first_two_chunks]),
        Err(CipherError::Authentication(1))
    ));
}

#[test]
fn decrypt_rejects_every_truncation_of_small_blob() {
    let blob = encrypt(&chacha(), b"a USB stick you can send");
    for length in 0..blob.len() {
        assert!(decrypt(&chacha(), &blob[..length]).is_err(), "{length}");
    }
}

#[test]
fn decrypt_rejects_trailing_bytes() {
    for size in [5, CHUNK_SIZE] {
        let mut blob = encrypt(&chacha(), &pattern(size));
        blob.push(0);
        assert!(decrypt(&chacha(), &blob).is_err(), "size {size}");
    }
}

#[test]
fn decrypt_rejects_swapped_chunks() {
    let plaintext = pattern(3 * CHUNK_SIZE);
    let blob = encrypt(&chacha(), &plaintext);
    let sealed = CHUNK_SIZE + TAG;
    let chunk = |i: usize| &blob[CHACHA_HEADER + i * sealed..CHACHA_HEADER + (i + 1) * sealed];
    let swapped = [&blob[..CHACHA_HEADER], chunk(1), chunk(0), chunk(2)].concat();
    assert!(matches!(
        decrypt(&chacha(), &swapped),
        Err(CipherError::Authentication(0))
    ));
}

#[test]
fn decrypt_rejects_chunk_from_another_blob() {
    // Two blobs of the same length but different content: splice chunk 0 of one
    // into the other. The nonce prefixes differ, so the chunk does not open.
    let first = encrypt(&chacha(), &pattern(2 * CHUNK_SIZE));
    let mut other_plaintext = pattern(2 * CHUNK_SIZE);
    other_plaintext[CHUNK_SIZE] ^= 1;
    let mut second = encrypt(&chacha(), &other_plaintext);
    let chunk_zero = CHACHA_HEADER..CHACHA_HEADER + CHUNK_SIZE + TAG;
    second[chunk_zero.clone()].copy_from_slice(&first[chunk_zero]);
    assert!(matches!(
        decrypt(&chacha(), &second),
        Err(CipherError::Authentication(0))
    ));
}

#[test]
fn decrypt_never_panics_on_random_input() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x000B_10B5);
    for _ in 0..2_000 {
        let length = rng.gen_range(0..200);
        let mut blob: Vec<u8> = (0..length).map(|_| rng.gen()).collect();
        if blob.len() >= 2 {
            blob[0] = BLOB_FORMAT_VERSION;
            blob[1] = rng.gen_range(0..3);
        }
        let _ = decrypt(&chacha(), &blob);
        let _ = decrypt(&NoCipher, &blob);
    }
}

#[test]
fn blob_len_matches_what_encrypt_writes() {
    let ciphers: [&dyn Cipher; 2] = [&NoCipher, &chacha()];
    for cipher in ciphers {
        for size in SIZES {
            let blob = encrypt(cipher, &pattern(size));
            assert_eq!(
                blob_len(cipher, size as u64),
                blob.len() as u64,
                "size {size}"
            );
        }
    }
}
