//! The blob format: a header, then the plaintext sealed in chunks.
//!
//! ```text
//! blob   = header || chunk_0 || chunk_1 || ... || chunk_n
//! header = version (u8) || cipher id (u8) || nonce prefix (cipher-specific length)
//! ```
//!
//! Every chunk holds [`CHUNK_SIZE`] plaintext bytes except the last, which holds
//! 1..=[`CHUNK_SIZE`] bytes (0 only when the whole plaintext is empty). The BLAKE3
//! hash of a blob covers the header too. See `docs/protocol.md`.

use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::cipher::{Cipher, CipherError, CipherId};

/// Version of the blob format written by [`encrypt_blob`].
pub const BLOB_FORMAT_VERSION: u8 = 1;

/// Plaintext bytes per chunk.
pub const CHUNK_SIZE: usize = 64 * 1024;

/// The first bytes of every blob. Every chunk is authenticated against all of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobHeader {
    version: u8,
    cipher: CipherId,
    nonce_prefix: Vec<u8>,
}

impl BlobHeader {
    /// A header in the current format version.
    pub fn new(cipher: CipherId, nonce_prefix: Vec<u8>) -> Self {
        Self {
            version: BLOB_FORMAT_VERSION,
            cipher,
            nonce_prefix,
        }
    }

    pub fn version(&self) -> u8 {
        self.version
    }

    pub fn cipher(&self) -> CipherId {
        self.cipher
    }

    pub fn nonce_prefix(&self) -> &[u8] {
        &self.nonce_prefix
    }

    /// The header as stored at the start of the blob. Also the AAD of every chunk.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(2 + self.nonce_prefix.len());
        bytes.push(self.version);
        bytes.push(self.cipher.to_byte());
        bytes.extend_from_slice(&self.nonce_prefix);
        bytes
    }

    /// Reads a header written for `cipher` from the start of a blob.
    pub fn read_from(reader: &mut impl Read, cipher: &dyn Cipher) -> Result<Self, CipherError> {
        let mut fixed = [0u8; 2];
        read_exact_or_truncated(reader, &mut fixed)?;
        let [version, cipher_byte] = fixed;
        if version != BLOB_FORMAT_VERSION {
            return Err(CipherError::UnsupportedVersion(version));
        }
        let found = CipherId::from_byte(cipher_byte)?;
        if found != cipher.id() {
            return Err(CipherError::CipherMismatch {
                expected: cipher.id(),
                found,
            });
        }
        let mut nonce_prefix = vec![0u8; cipher.nonce_prefix_len()];
        read_exact_or_truncated(reader, &mut nonce_prefix)?;
        Ok(Self {
            version,
            cipher: found,
            nonce_prefix,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_version_for_test(&self, version: u8) -> Self {
        Self {
            version,
            ..self.clone()
        }
    }
}

/// Sizes of a blob written by [`encrypt_blob`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobInfo {
    pub plaintext_len: u64,
    pub blob_len: u64,
}

/// Encrypts `plaintext` into a blob written to `out`.
///
/// Reads `plaintext` twice: once to hash it and derive the nonce prefix, once to
/// encrypt. The second pass re-hashes the input and fails with
/// [`CipherError::PlaintextChanged`] if it differs from the first, because then
/// the nonce prefix no longer belongs to the encrypted bytes. On that error the
/// caller must discard everything written to `out`, and must never have sent any
/// of it to a peer.
pub fn encrypt_blob<R: Read + Seek>(
    cipher: &dyn Cipher,
    plaintext: &mut R,
    out: &mut impl Write,
) -> Result<BlobInfo, CipherError> {
    let start = plaintext.stream_position()?;
    let mut first_pass = blake3::Hasher::new();
    first_pass.update_reader(&mut *plaintext)?;
    plaintext.seek(SeekFrom::Start(start))?;

    let header = BlobHeader::new(
        cipher.id(),
        cipher.derive_nonce_prefix(&first_pass.finalize()),
    );
    let header_bytes = header.to_bytes();
    out.write_all(&header_bytes)?;
    let mut blob_len = header_bytes.len() as u64;

    let mut second_pass = blake3::Hasher::new();
    let mut chunks = ChunkReader::new(plaintext, CHUNK_SIZE)?;
    let mut index = 0u32;
    loop {
        let (chunk, is_last) = chunks.current();
        second_pass.update(chunk);
        let sealed = cipher.seal_chunk(&header, index, is_last, chunk)?;
        out.write_all(&sealed)?;
        blob_len += sealed.len() as u64;
        if is_last {
            break;
        }
        chunks.advance()?;
        index = index.checked_add(1).ok_or(CipherError::TooManyChunks)?;
    }

    if second_pass.finalize() != first_pass.finalize() {
        return Err(CipherError::PlaintextChanged);
    }
    Ok(BlobInfo {
        plaintext_len: second_pass.count(),
        blob_len,
    })
}

/// Decrypts a blob read from `blob`, writing the plaintext to `out`.
/// Returns the number of plaintext bytes written.
///
/// Chunks are written as soon as they are authenticated. If this fails, what was
/// already written is authentic but incomplete.
pub fn decrypt_blob(
    cipher: &dyn Cipher,
    blob: &mut impl Read,
    out: &mut impl Write,
) -> Result<u64, CipherError> {
    let header = BlobHeader::read_from(blob, cipher)?;
    let mut chunks = ChunkReader::new(blob, CHUNK_SIZE + cipher.tag_len())?;
    let mut plaintext_len = 0u64;
    let mut index = 0u32;
    loop {
        let (chunk, is_last) = chunks.current();
        if chunk.len() < cipher.tag_len() {
            return Err(CipherError::Truncated);
        }
        let opened = cipher.open_chunk(&header, index, is_last, chunk)?;
        out.write_all(&opened)?;
        plaintext_len += opened.len() as u64;
        if is_last {
            return Ok(plaintext_len);
        }
        chunks.advance()?;
        index = index.checked_add(1).ok_or(CipherError::TooManyChunks)?;
    }
}

/// Reads fixed-size chunks and knows whether the current one is the last,
/// by reading one chunk ahead. A chunk shorter than the full size is always last.
struct ChunkReader<'a, R: Read> {
    reader: &'a mut R,
    current: Vec<u8>,
    current_len: usize,
    next: Vec<u8>,
    next_len: usize,
}

impl<'a, R: Read> ChunkReader<'a, R> {
    fn new(reader: &'a mut R, chunk_size: usize) -> io::Result<Self> {
        let mut chunks = Self {
            reader,
            current: vec![0; chunk_size],
            current_len: 0,
            next: vec![0; chunk_size],
            next_len: 0,
        };
        chunks.current_len = read_full(chunks.reader, &mut chunks.current)?;
        chunks.read_next()?;
        Ok(chunks)
    }

    fn read_next(&mut self) -> io::Result<()> {
        self.next_len = if self.current_len == self.current.len() {
            read_full(self.reader, &mut self.next)?
        } else {
            0
        };
        Ok(())
    }

    fn current(&self) -> (&[u8], bool) {
        (&self.current[..self.current_len], self.next_len == 0)
    }

    fn advance(&mut self) -> io::Result<()> {
        std::mem::swap(&mut self.current, &mut self.next);
        self.current_len = self.next_len;
        self.read_next()
    }
}

/// Reads until `buffer` is full or the reader is at EOF. Returns the bytes read.
fn read_full(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

fn read_exact_or_truncated(reader: &mut impl Read, buffer: &mut [u8]) -> Result<(), CipherError> {
    if read_full(reader, buffer)? == buffer.len() {
        Ok(())
    } else {
        Err(CipherError::Truncated)
    }
}

#[cfg(test)]
mod tests;
