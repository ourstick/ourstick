//! The identity cipher: chunks are stored as plaintext.
//!
//! Used for tests and for sticks whose owner chose no encryption. The blob
//! still carries a header, so the format is the same as for real ciphers.

use super::{Cipher, CipherError, CipherId};
use crate::blob::BlobHeader;

/// A [`Cipher`] that does not encrypt. It offers no confidentiality and no
/// authentication beyond the BLAKE3 hash of the blob.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCipher;

impl Cipher for NoCipher {
    fn id(&self) -> CipherId {
        CipherId::None
    }

    fn nonce_prefix_len(&self) -> usize {
        0
    }

    fn tag_len(&self) -> usize {
        0
    }

    fn derive_nonce_prefix(&self, _plaintext_hash: &blake3::Hash) -> Vec<u8> {
        Vec::new()
    }

    fn seal_chunk(
        &self,
        _header: &BlobHeader,
        _index: u32,
        _is_last: bool,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CipherError> {
        Ok(plaintext.to_vec())
    }

    fn open_chunk(
        &self,
        _header: &BlobHeader,
        _index: u32,
        _is_last: bool,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, CipherError> {
        Ok(ciphertext.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_cipher_describes_itself() {
        assert_eq!(NoCipher.id(), CipherId::None);
        assert_eq!(NoCipher.nonce_prefix_len(), 0);
        assert_eq!(NoCipher.tag_len(), 0);
        assert!(NoCipher.derive_nonce_prefix(&blake3::hash(b"x")).is_empty());
    }

    #[test]
    fn no_cipher_chunks_are_plaintext() {
        let header = BlobHeader::new(CipherId::None, Vec::new());
        let sealed = NoCipher.seal_chunk(&header, 0, true, b"hello").unwrap();
        assert_eq!(sealed, b"hello");
        let opened = NoCipher.open_chunk(&header, 0, true, &sealed).unwrap();
        assert_eq!(opened, b"hello");
    }
}
