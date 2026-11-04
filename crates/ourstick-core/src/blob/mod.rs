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
    /// BLAKE3 of the plaintext, from which the nonce prefix was derived.
    pub plaintext_hash: [u8; 32],
}

/// The exact length of the blob that [`encrypt_blob`] writes for a plaintext of
/// `plaintext_len` bytes: header, plaintext, and one tag per chunk (at least one).
pub fn blob_len(cipher: &dyn Cipher, plaintext_len: u64) -> u64 {
    let chunks = plaintext_len.div_ceil(CHUNK_SIZE as u64).max(1);
    2 + cipher.nonce_prefix_len() as u64 + plaintext_len + chunks * cipher.tag_len() as u64
}

/// The longest a blob of a `plaintext_len`-byte file may be, whatever its
/// cipher: what a reader fetches at most for a manifest entry of that size.
/// The size is the owner's word, and the blob's hash covers whatever it sends;
/// without this bound, an entry of 1 byte could bring a blob of any size.
pub fn largest_blob_len(plaintext_len: u64) -> u64 {
    use crate::cipher::chacha::{NONCE_PREFIX_LEN, TAG_LEN};
    let chunks = plaintext_len.div_ceil(CHUNK_SIZE as u64).max(1);
    (2 + NONCE_PREFIX_LEN as u64)
        .saturating_add(plaintext_len)
        .saturating_add(chunks.saturating_mul(TAG_LEN as u64))
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
    encrypt_with(cipher, plaintext, out, first_pass.finalize())
}

/// Like [`encrypt_blob`], reading `plaintext` once: the nonce prefix comes
/// from `plaintext_hash`, the hash an earlier read gave (the owner's
/// catalogue, ADR 0007), and the read here only checks it. If the plaintext
/// is no longer what was hashed, this fails with
/// [`CipherError::PlaintextChanged`] after writing a blob whose nonce belongs
/// to other bytes: the caller must then delete everything written to `out`
/// at once, and must never let any of it leave the machine. Blobs made here
/// are served only if their hash is the manifest's (ADR 0007), which such a
/// blob never has.
pub fn encrypt_blob_hashed_before(
    cipher: &dyn Cipher,
    plaintext: &mut impl Read,
    out: &mut impl Write,
    plaintext_hash: &[u8; 32],
) -> Result<BlobInfo, CipherError> {
    encrypt_with(
        cipher,
        plaintext,
        out,
        blake3::Hash::from_bytes(*plaintext_hash),
    )
}

/// Encrypts with the nonce prefix of `plaintext_hash`, and checks that the
/// bytes read hash to it.
fn encrypt_with(
    cipher: &dyn Cipher,
    plaintext: &mut impl Read,
    out: &mut impl Write,
    plaintext_hash: blake3::Hash,
) -> Result<BlobInfo, CipherError> {
    let mut blob = BlobReader::new(cipher, plaintext, plaintext_hash)?;
    io::copy(&mut blob, out).map_err(cipher_error)?;
    blob.info()
        .ok_or_else(|| CipherError::Io(io::Error::other("the blob was not read to its end")))
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

pub mod made;
mod stream;
#[cfg(test)]
mod tests;

pub use stream::{cipher_error, BlobReader};
