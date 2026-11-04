//! XChaCha20-Poly1305 with a nonce prefix derived from the plaintext.
//!
//! Nonce of chunk `i`: `prefix (19 bytes) || i (u32 big-endian) || last (1 byte)`.
//! AAD of every chunk: the full blob header. See `docs/protocol.md`.

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use super::{Cipher, CipherError, CipherId, StickKey};
use crate::blob::BlobHeader;

/// BLAKE3 `derive_key` context for the AEAD key.
pub const AEAD_KEY_CONTEXT: &str = "OurStick 2026-09-28 blob XChaCha20-Poly1305 key";
/// BLAKE3 `derive_key` context for the key that derives nonce prefixes.
pub const NONCE_KEY_CONTEXT: &str = "OurStick 2026-09-28 blob nonce prefix key";

pub(crate) const NONCE_PREFIX_LEN: usize = 19;
pub(crate) const TAG_LEN: usize = 16;

/// A [`Cipher`] using XChaCha20-Poly1305. Both of its keys are derived from the
/// [`StickKey`], so the stick key itself never touches the AEAD or the nonce.
#[derive(Clone)]
pub struct ChaChaCipher {
    aead: XChaCha20Poly1305,
    nonce_key: [u8; 32],
}

impl ChaChaCipher {
    pub fn new(key: &StickKey) -> Self {
        let aead_key = blake3::derive_key(AEAD_KEY_CONTEXT, key.as_bytes());
        let nonce_key = blake3::derive_key(NONCE_KEY_CONTEXT, key.as_bytes());
        Self {
            aead: XChaCha20Poly1305::new(&aead_key.into()),
            nonce_key,
        }
    }
}

impl fmt::Debug for ChaChaCipher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ChaChaCipher(<redacted>)")
    }
}

/// Builds the 24-byte nonce of one chunk. `prefix` must be 19 bytes long.
fn chunk_nonce(prefix: &[u8], index: u32, is_last: bool) -> XNonce {
    let mut nonce = [0u8; 24];
    nonce[..NONCE_PREFIX_LEN].copy_from_slice(prefix);
    nonce[NONCE_PREFIX_LEN..23].copy_from_slice(&index.to_be_bytes());
    nonce[23] = u8::from(is_last);
    nonce.into()
}

fn check_prefix_len(header: &BlobHeader) -> Result<(), CipherError> {
    let found = header.nonce_prefix().len();
    if found == NONCE_PREFIX_LEN {
        Ok(())
    } else {
        Err(CipherError::InvalidNoncePrefix {
            expected: NONCE_PREFIX_LEN,
            found,
        })
    }
}

impl Cipher for ChaChaCipher {
    fn id(&self) -> CipherId {
        CipherId::XChaCha20Poly1305
    }

    fn nonce_prefix_len(&self) -> usize {
        NONCE_PREFIX_LEN
    }

    fn tag_len(&self) -> usize {
        TAG_LEN
    }

    fn derive_nonce_prefix(&self, plaintext_hash: &blake3::Hash) -> Vec<u8> {
        let derived = blake3::keyed_hash(&self.nonce_key, plaintext_hash.as_bytes());
        derived.as_bytes()[..NONCE_PREFIX_LEN].to_vec()
    }

    fn seal_chunk(
        &self,
        header: &BlobHeader,
        index: u32,
        is_last: bool,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CipherError> {
        check_prefix_len(header)?;
        let nonce = chunk_nonce(header.nonce_prefix(), index, is_last);
        let aad = header.to_bytes();
        let payload = Payload {
            msg: plaintext,
            aad: &aad,
        };
        // Sealing only fails if the plaintext exceeds the AEAD limit (256 GiB),
        // which a chunk never does.
        self.aead
            .encrypt(&nonce, payload)
            .map_err(|_| CipherError::Authentication(index))
    }

    fn open_chunk(
        &self,
        header: &BlobHeader,
        index: u32,
        is_last: bool,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, CipherError> {
        check_prefix_len(header)?;
        let nonce = chunk_nonce(header.nonce_prefix(), index, is_last);
        let aad = header.to_bytes();
        let payload = Payload {
            msg: ciphertext,
            aad: &aad,
        };
        self.aead
            .decrypt(&nonce, payload)
            .map_err(|_| CipherError::Authentication(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> ChaChaCipher {
        ChaChaCipher::new(&StickKey::from_bytes([1; 32]))
    }

    fn header(cipher: &ChaChaCipher, plaintext: &[u8]) -> BlobHeader {
        let prefix = cipher.derive_nonce_prefix(&blake3::hash(plaintext));
        BlobHeader::new(cipher.id(), prefix)
    }

    #[test]
    fn chacha_describes_itself() {
        let cipher = cipher();
        assert_eq!(cipher.id(), CipherId::XChaCha20Poly1305);
        assert_eq!(cipher.nonce_prefix_len(), 19);
        assert_eq!(cipher.tag_len(), 16);
    }

    #[test]
    fn debug_does_not_print_keys() {
        assert_eq!(format!("{:?}", cipher()), "ChaChaCipher(<redacted>)");
    }

    #[test]
    fn nonce_prefix_is_deterministic_and_depends_on_key_and_plaintext() {
        let hash = blake3::hash(b"same");
        let first = cipher().derive_nonce_prefix(&hash);
        assert_eq!(first.len(), 19);
        assert_eq!(first, cipher().derive_nonce_prefix(&hash));
        assert_ne!(first, cipher().derive_nonce_prefix(&blake3::hash(b"other")));
        let other_key = ChaChaCipher::new(&StickKey::from_bytes([2; 32]));
        assert_ne!(first, other_key.derive_nonce_prefix(&hash));
    }

    #[test]
    fn chunk_nonce_layout() {
        let prefix: Vec<u8> = (0..19).collect();
        let nonce = chunk_nonce(&prefix, 0x0102_0304, true);
        assert_eq!(&nonce[..19], &prefix[..]);
        assert_eq!(&nonce[19..23], &[1, 2, 3, 4]);
        assert_eq!(nonce[23], 1);
        assert_eq!(chunk_nonce(&prefix, 0, false)[23], 0);
    }

    #[test]
    fn seal_then_open_roundtrips() {
        let cipher = cipher();
        let header = header(&cipher, b"hello");
        let sealed = cipher.seal_chunk(&header, 0, true, b"hello").unwrap();
        assert_eq!(sealed.len(), 5 + 16);
        assert_eq!(
            cipher.open_chunk(&header, 0, true, &sealed).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn open_rejects_tampered_chunk() {
        let cipher = cipher();
        let header = header(&cipher, b"hello");
        let mut sealed = cipher.seal_chunk(&header, 0, true, b"hello").unwrap();
        sealed[0] ^= 1;
        assert!(matches!(
            cipher.open_chunk(&header, 0, true, &sealed),
            Err(CipherError::Authentication(0))
        ));
    }

    #[test]
    fn open_rejects_wrong_index_or_last_flag() {
        let cipher = cipher();
        let header = header(&cipher, b"hello");
        let sealed = cipher.seal_chunk(&header, 3, false, b"hello").unwrap();
        assert!(cipher.open_chunk(&header, 4, false, &sealed).is_err());
        assert!(cipher.open_chunk(&header, 3, true, &sealed).is_err());
        assert!(cipher.open_chunk(&header, 3, false, &sealed).is_ok());
    }

    #[test]
    fn open_rejects_chunk_moved_to_blob_with_other_header() {
        let cipher = cipher();
        let original = header(&cipher, b"hello");
        let sealed = cipher.seal_chunk(&original, 0, true, b"hello").unwrap();

        // Same nonce prefix, different format version byte: only the AAD differs.
        let other = original.with_version_for_test(original.version() + 1);
        assert!(cipher.open_chunk(&other, 0, true, &sealed).is_err());
    }

    #[test]
    fn open_rejects_wrong_key() {
        let header = header(&cipher(), b"hello");
        let sealed = cipher().seal_chunk(&header, 0, true, b"hello").unwrap();
        let other_key = ChaChaCipher::new(&StickKey::from_bytes([2; 32]));
        assert!(other_key.open_chunk(&header, 0, true, &sealed).is_err());
    }

    #[test]
    fn seal_and_open_reject_wrong_prefix_length() {
        let cipher = cipher();
        let header = BlobHeader::new(cipher.id(), vec![0; 3]);
        let wrong_length = CipherError::InvalidNoncePrefix {
            expected: 19,
            found: 3,
        };
        let sealed = cipher.seal_chunk(&header, 0, true, b"x").unwrap_err();
        assert_eq!(sealed.to_string(), wrong_length.to_string());
        let opened = cipher.open_chunk(&header, 0, true, &[0; 16]).unwrap_err();
        assert_eq!(opened.to_string(), wrong_length.to_string());
    }
}
