//! A blob read as it is made: [`BlobReader`] gives the bytes `encrypt_blob`
//! writes, in order, from a plaintext reader. The owner's catalogue hashes
//! it into the blob's hash tree (ADR 0017, rule 1); `encrypt_blob` copies it.

use std::io::{self, Read};

use super::{BlobHeader, BlobInfo, ChunkReader, CHUNK_SIZE};
use crate::cipher::{Cipher, CipherError};

/// The blob of `plaintext`, read in order. The nonce prefix comes from
/// `plaintext_hash`; the plaintext is hashed again as it is read, and if it
/// is not what was hashed, the read that reaches its end fails with
/// [`CipherError::PlaintextChanged`] (inside an `io::Error`, see
/// [`cipher_error`]). Whatever was read of such a blob must be discarded and
/// never leave the machine: its nonce belongs to other bytes (ADR 0003).
pub struct BlobReader<'a, R: Read> {
    cipher: &'a dyn Cipher,
    header: BlobHeader,
    chunks: ChunkReader<'a, R>,
    plaintext_hash: blake3::Hash,
    second_pass: blake3::Hasher,
    /// Bytes made and not read yet, from `at`.
    pending: Vec<u8>,
    at: usize,
    index: u32,
    started: bool,
    /// The last chunk was sealed.
    sealed_all: bool,
    blob_len: u64,
    checked: bool,
}

impl<'a, R: Read> BlobReader<'a, R> {
    pub fn new(
        cipher: &'a dyn Cipher,
        plaintext: &'a mut R,
        plaintext_hash: blake3::Hash,
    ) -> io::Result<Self> {
        let header = BlobHeader::new(cipher.id(), cipher.derive_nonce_prefix(&plaintext_hash));
        Ok(Self {
            cipher,
            header,
            chunks: ChunkReader::new(plaintext, CHUNK_SIZE)?,
            plaintext_hash,
            second_pass: blake3::Hasher::new(),
            pending: Vec::new(),
            at: 0,
            index: 0,
            started: false,
            sealed_all: false,
            blob_len: 0,
            checked: false,
        })
    }

    /// What was made, once the whole blob was read and checked.
    pub fn info(&self) -> Option<BlobInfo> {
        self.checked.then(|| BlobInfo {
            plaintext_len: self.second_pass.count(),
            blob_len: self.blob_len,
            plaintext_hash: *self.plaintext_hash.as_bytes(),
        })
    }

    /// Makes the next bytes into `pending`; false at the end.
    fn make_more(&mut self) -> io::Result<bool> {
        if !self.started {
            self.started = true;
            self.pending = self.header.to_bytes();
            return Ok(true);
        }
        if self.sealed_all {
            if !self.checked {
                if self.second_pass.finalize() != self.plaintext_hash {
                    return Err(io::Error::other(CipherError::PlaintextChanged));
                }
                self.checked = true;
            }
            return Ok(false);
        }
        let (chunk, is_last) = self.chunks.current();
        self.second_pass.update(chunk);
        self.pending = self
            .cipher
            .seal_chunk(&self.header, self.index, is_last, chunk)
            .map_err(io::Error::other)?;
        if is_last {
            self.sealed_all = true;
        } else {
            self.chunks.advance()?;
            self.index = self
                .index
                .checked_add(1)
                .ok_or_else(|| io::Error::other(CipherError::TooManyChunks))?;
        }
        Ok(true)
    }
}

impl<R: Read> Read for BlobReader<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.at == self.pending.len() {
            self.at = 0;
            self.pending.clear();
            if !self.make_more()? {
                return Ok(0);
            }
        }
        let n = (self.pending.len() - self.at).min(out.len());
        out[..n].copy_from_slice(&self.pending[self.at..self.at + n]);
        self.at += n;
        self.blob_len += n as u64;
        Ok(n)
    }
}

/// The [`CipherError`] inside an `io::Error` from a [`BlobReader`] (or the
/// I/O error itself).
pub fn cipher_error(error: io::Error) -> CipherError {
    match error
        .get_ref()
        .is_some_and(|inner| inner.is::<CipherError>())
    {
        true => *error
            .into_inner()
            .and_then(|inner| inner.downcast::<CipherError>().ok())
            .expect("checked just above"),
        false => CipherError::Io(error),
    }
}
