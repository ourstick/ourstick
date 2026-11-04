//! Encryption of blobs, one chunk at a time.
//!
//! A [`Cipher`] seals and opens single chunks. The blob layout around the chunks
//! (header, chunking, last-chunk flag) lives in [`crate::blob`], so every cipher
//! shares it. See `docs/protocol.md` for the byte layout and
//! `docs/decisions/0003-deterministic-nonce.md` for why nonces are derived.

pub(crate) mod chacha;
mod no_cipher;

use std::fmt;
use std::io;

use rand::RngCore;

use crate::blob::BlobHeader;

pub use chacha::{ChaChaCipher, AEAD_KEY_CONTEXT, NONCE_KEY_CONTEXT};
pub use no_cipher::NoCipher;

/// Identifies the cipher a blob was written with. Stored in every blob header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CipherId {
    /// No encryption: chunks are the plaintext.
    None = 0,
    /// XChaCha20-Poly1305 with a derived nonce prefix.
    XChaCha20Poly1305 = 1,
}

impl CipherId {
    /// Parses the byte stored in a blob header.
    pub fn from_byte(byte: u8) -> Result<Self, CipherError> {
        match byte {
            0 => Ok(Self::None),
            1 => Ok(Self::XChaCha20Poly1305),
            other => Err(CipherError::UnknownCipher(other)),
        }
    }

    /// The byte stored in a blob header.
    pub fn to_byte(self) -> u8 {
        self as u8
    }
}

/// The cipher a stick uses, keyed with its `StickKey`.
pub fn cipher_for(id: CipherId, key: &StickKey) -> std::sync::Arc<dyn Cipher> {
    match id {
        CipherId::None => std::sync::Arc::new(NoCipher),
        CipherId::XChaCha20Poly1305 => std::sync::Arc::new(ChaChaCipher::new(key)),
    }
}

/// The secret of a stick. Whoever holds it can read the stick.
///
/// `Debug` never prints the bytes, so a key cannot leak through logs.
#[derive(Clone, PartialEq, Eq)]
pub struct StickKey([u8; 32]);

impl StickKey {
    /// Creates a new random key from the operating system's secure RNG.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for StickKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StickKey(<redacted>)")
    }
}

/// Seals and opens the chunks of a blob.
///
/// Implementations bind every chunk to the full blob header (as AAD), to its
/// index and to whether it is the last one, so chunks cannot be reordered,
/// dropped, or moved into another blob.
pub trait Cipher: Send + Sync {
    fn id(&self) -> CipherId;

    /// Length of the nonce prefix this cipher stores in the blob header.
    fn nonce_prefix_len(&self) -> usize;

    /// Bytes each sealed chunk adds on top of its plaintext.
    fn tag_len(&self) -> usize;

    /// Derives the nonce prefix of a blob from the BLAKE3 hash of its plaintext.
    /// The same plaintext and key always give the same prefix.
    fn derive_nonce_prefix(&self, plaintext_hash: &blake3::Hash) -> Vec<u8>;

    /// Seals chunk number `index` of the blob described by `header`.
    fn seal_chunk(
        &self,
        header: &BlobHeader,
        index: u32,
        is_last: bool,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CipherError>;

    /// Opens chunk number `index`, failing if it was altered or does not belong
    /// at this position of this blob.
    fn open_chunk(
        &self,
        header: &BlobHeader,
        index: u32,
        is_last: bool,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, CipherError>;
}

/// Errors produced while encrypting or decrypting a blob.
#[derive(Debug, thiserror::Error)]
pub enum CipherError {
    #[error("unsupported blob format version {0}")]
    UnsupportedVersion(u8),
    #[error("unknown cipher id {0}")]
    UnknownCipher(u8),
    #[error("blob was written with cipher {found:?}, expected {expected:?}")]
    CipherMismatch { expected: CipherId, found: CipherId },
    #[error("blob is truncated")]
    Truncated,
    #[error("nonce prefix has length {found}, expected {expected}")]
    InvalidNoncePrefix { expected: usize, found: usize },
    #[error("plaintext changed while it was being encrypted")]
    PlaintextChanged,
    #[error("chunk {0} failed authentication")]
    Authentication(u32),
    #[error("blob has more chunks than the format allows")]
    TooManyChunks,
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cipher_id_roundtrips_through_its_byte() {
        for id in [CipherId::None, CipherId::XChaCha20Poly1305] {
            assert_eq!(CipherId::from_byte(id.to_byte()).unwrap(), id);
        }
        assert_eq!(CipherId::None.to_byte(), 0);
        assert_eq!(CipherId::XChaCha20Poly1305.to_byte(), 1);
    }

    #[test]
    fn cipher_for_picks_the_named_cipher() {
        let key = StickKey::from_bytes([1; 32]);
        for id in [CipherId::None, CipherId::XChaCha20Poly1305] {
            assert_eq!(cipher_for(id, &key).id(), id);
        }
    }

    #[test]
    fn cipher_id_rejects_unknown_byte() {
        assert!(matches!(
            CipherId::from_byte(2),
            Err(CipherError::UnknownCipher(2))
        ));
    }

    #[test]
    fn generated_keys_differ() {
        assert_ne!(StickKey::generate(), StickKey::generate());
    }

    #[test]
    fn key_roundtrips_through_bytes() {
        let key = StickKey::from_bytes([7; 32]);
        assert_eq!(key.as_bytes(), &[7; 32]);
    }

    #[test]
    fn key_debug_does_not_print_bytes() {
        let key = StickKey::from_bytes([0xAB; 32]);
        let printed = format!("{key:?}");
        assert!(!printed.to_lowercase().contains("ab"), "{printed}");
        assert!(!printed.contains("171"), "{printed}");
    }
}
