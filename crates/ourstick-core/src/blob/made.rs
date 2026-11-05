//! A blob made from its file when its bytes are asked for (ADR 0017): any
//! range of what [`super::encrypt_blob`] would write, read and encrypted
//! there and then. The nonce prefix comes from the plaintext hash recorded
//! by the catalogue, so these bytes equal the blob that was hashed only while
//! the file is unchanged; whoever sends them must check every part against
//! the blob's hash tree first (ADR 0017, rule 3).

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::{blob_len, BlobHeader, CHUNK_SIZE};
use crate::cipher::Cipher;
use crate::sender_files::{FileFacts, ReadAt, SenderFiles};

/// How a made blob's file is read: through a disk's turns for transfers
/// (ADR 0017, amendment 1).
pub trait FileReads: Send + Sync {
    /// Runs `read`, which reads from the file and says how many bytes;
    /// `begins`: the blob's first read. Blocking.
    fn read(&self, begins: bool, read: &mut dyn FnMut() -> io::Result<u64>) -> io::Result<()>;
}

/// One file's blob, made on demand.
pub struct MadeBlob {
    file: Box<dyn ReadAt>,
    cipher: Arc<dyn Cipher>,
    header: BlobHeader,
    header_bytes: Vec<u8>,
    /// The plaintext size the catalogue recorded: bytes past it are never
    /// read, and a file shorter than it is an error.
    plaintext_len: u64,
    /// The last chunk made, by index: a 16 KiB group in the middle of a
    /// 64 KiB chunk does not make it again. Held while a chunk is made, so
    /// reads of one blob run one at a time (one transfer reads one blob, in
    /// order).
    last: Mutex<Option<(u64, Arc<Vec<u8>>)>>,
    /// Through what the file is read; directly if none.
    reads: Option<Arc<dyn FileReads>>,
    begun: AtomicBool,
}

impl MadeBlob {
    /// The blob of the file at `path`, read through `files`, of
    /// `plaintext_len` bytes whose BLAKE3 is `plaintext_hash`, as the
    /// catalogue recorded them. Blocking.
    pub fn open(
        files: &dyn SenderFiles,
        path: &Path,
        below: usize,
        cipher: Arc<dyn Cipher>,
        plaintext_len: u64,
        plaintext_hash: &[u8; 32],
    ) -> io::Result<Self> {
        let file = files.open_below(path, below)?;
        let header = BlobHeader::new(
            cipher.id(),
            cipher.derive_nonce_prefix(&blake3::Hash::from_bytes(*plaintext_hash)),
        );
        let header_bytes = header.to_bytes();
        Ok(Self {
            file,
            cipher,
            header,
            header_bytes,
            plaintext_len,
            last: Mutex::new(None),
            reads: None,
            begun: AtomicBool::new(false),
        })
    }

    /// Its file read through `reads` from now on.
    pub fn with_reads(mut self, reads: Arc<dyn FileReads>) -> Self {
        self.reads = Some(reads);
        self
    }

    /// Its file's facts, as opened.
    pub fn facts(&self) -> io::Result<FileFacts> {
        self.file.facts()
    }

    /// The blob's length, from the recorded size.
    pub fn len(&self) -> u64 {
        blob_len(self.cipher.as_ref(), self.plaintext_len)
    }

    /// Never: a blob has at least its header.
    pub fn is_empty(&self) -> bool {
        false
    }

    fn chunks(&self) -> u64 {
        self.plaintext_len.div_ceil(CHUNK_SIZE as u64).max(1)
    }

    fn sealed_len(&self) -> u64 {
        (CHUNK_SIZE + self.cipher.tag_len()) as u64
    }

    /// Chunk `index`, sealed.
    fn chunk(&self, index: u64) -> io::Result<Arc<Vec<u8>>> {
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((at, sealed)) = last.as_ref() {
            if *at == index {
                return Ok(sealed.clone());
            }
        }
        let start = index * CHUNK_SIZE as u64;
        let len = (self.plaintext_len - start).min(CHUNK_SIZE as u64) as usize;
        let mut plaintext = vec![0u8; len];
        // Only a chunk not cached reads the file (and takes a turn).
        match &self.reads {
            Some(reads) => {
                let begins = !self.begun.swap(true, Ordering::Relaxed);
                reads.read(begins, &mut || {
                    self.file
                        .read_exact_at(&mut plaintext, start)
                        .map(|()| len as u64)
                })?;
            }
            None => self.file.read_exact_at(&mut plaintext, start)?,
        }
        let is_last = index + 1 == self.chunks();
        let index32 = u32::try_from(index).map_err(io::Error::other)?;
        let sealed = self
            .cipher
            .seal_chunk(&self.header, index32, is_last, &plaintext)
            .map_err(io::Error::other)?;
        let sealed = Arc::new(sealed);
        *last = Some((index, sealed.clone()));
        Ok(sealed)
    }

    /// The bytes `[offset, offset + out.len())` of the blob. Blocking.
    pub fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        let end = offset.checked_add(out.len() as u64);
        if end.is_none_or(|end| end > self.len()) {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "past the end of the blob",
            ));
        }
        let header_len = self.header_bytes.len() as u64;
        let mut done = 0usize;
        while done < out.len() {
            let at = offset + done as u64;
            if at < header_len {
                let from = at as usize;
                let n = (self.header_bytes.len() - from).min(out.len() - done);
                out[done..done + n].copy_from_slice(&self.header_bytes[from..from + n]);
                done += n;
                continue;
            }
            let in_chunks = at - header_len;
            let index = in_chunks / self.sealed_len();
            let within = (in_chunks % self.sealed_len()) as usize;
            let sealed = self.chunk(index)?;
            let n = (sealed.len() - within).min(out.len() - done);
            out[done..done + n].copy_from_slice(&sealed[within..within + n]);
            done += n;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::encrypt_blob;
    use crate::cipher::{ChaChaCipher, StickKey};

    fn cipher() -> Arc<dyn Cipher> {
        Arc::new(ChaChaCipher::new(&StickKey::from_bytes([5; 32])))
    }

    #[test]
    fn every_range_is_the_blob_encrypt_blob_writes() {
        let dir = tempfile::tempdir().unwrap();
        for size in [
            0usize,
            1,
            16 * 1024 + 1,
            CHUNK_SIZE - 1,
            CHUNK_SIZE,
            2 * CHUNK_SIZE + 7,
        ] {
            let plaintext: Vec<u8> = (0..size).map(|i| (i * 7 + 3) as u8).collect();
            let path = dir.path().join(format!("f{size}"));
            std::fs::write(&path, &plaintext).unwrap();
            let mut blob = Vec::new();
            let info = encrypt_blob(
                cipher().as_ref(),
                &mut io::Cursor::new(&plaintext),
                &mut blob,
            )
            .unwrap();
            let made = MadeBlob::open(
                &crate::sender_files::LocalFiles,
                &path,
                1,
                cipher(),
                size as u64,
                &info.plaintext_hash,
            )
            .unwrap();
            assert_eq!(made.len(), blob.len() as u64);
            for (offset, len) in [
                (0, blob.len()),
                (1, 30),
                (40, 16 * 1024),
                (blob.len() / 2, blob.len() / 2),
            ] {
                if offset > blob.len() {
                    continue;
                }
                let len = len.min(blob.len() - offset);
                let mut out = vec![0u8; len];
                made.read_at(offset as u64, &mut out).unwrap();
                assert_eq!(out, blob[offset..offset + len], "size {size} at {offset}");
            }
        }
    }

    #[test]
    fn a_changed_file_gives_other_bytes_which_the_sender_must_catch() {
        // The reader does not check: it makes what the file holds now, under
        // the recorded nonce prefix. Checking is the sender's (ADR 0017,
        // rule 3); this only shows there is something to catch.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        let plaintext = vec![9u8; 3 * CHUNK_SIZE];
        std::fs::write(&path, &plaintext).unwrap();
        let mut blob = Vec::new();
        let info = encrypt_blob(
            cipher().as_ref(),
            &mut io::Cursor::new(&plaintext),
            &mut blob,
        )
        .unwrap();
        let mut changed = plaintext.clone();
        changed[CHUNK_SIZE + 5] = 0;
        std::fs::write(&path, &changed).unwrap();
        let made = MadeBlob::open(
            &crate::sender_files::LocalFiles,
            &path,
            1,
            cipher(),
            plaintext.len() as u64,
            &info.plaintext_hash,
        )
        .unwrap();
        let mut out = vec![0u8; blob.len()];
        made.read_at(0, &mut out).unwrap();
        assert_ne!(out, blob);
        // The chunks before the change are the same bytes.
        let first = blob.len() / 3;
        assert_eq!(out[..first], blob[..first]);
    }

    #[test]
    fn bytes_past_the_recorded_size_are_never_read() {
        // A file that grew since: the blob is that of its first bytes only.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, b"hello, and more since").unwrap();
        let mut blob = Vec::new();
        let info =
            encrypt_blob(cipher().as_ref(), &mut io::Cursor::new(b"hello"), &mut blob).unwrap();
        let made = MadeBlob::open(
            &crate::sender_files::LocalFiles,
            &path,
            1,
            cipher(),
            5,
            &info.plaintext_hash,
        )
        .unwrap();
        let mut out = vec![0u8; blob.len()];
        made.read_at(0, &mut out).unwrap();
        assert_eq!(out, blob);
        let empty = MadeBlob::open(
            &crate::sender_files::LocalFiles,
            &path,
            1,
            cipher(),
            0,
            blake3::hash(b"").as_bytes(),
        )
        .unwrap();
        let mut blob = Vec::new();
        encrypt_blob(cipher().as_ref(), &mut io::Cursor::new(b""), &mut blob).unwrap();
        let mut out = vec![0u8; blob.len()];
        empty.read_at(0, &mut out).unwrap();
        assert_eq!(out, blob);
    }

    #[test]
    fn a_shorter_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, vec![1u8; 100]).unwrap();
        let made = MadeBlob::open(
            &crate::sender_files::LocalFiles,
            &path,
            1,
            cipher(),
            200,
            &[0; 32],
        )
        .unwrap();
        let mut out = vec![0u8; made.len() as usize];
        assert!(made.read_at(0, &mut out).is_err());
        // And nothing past the blob is made.
        assert!(made.read_at(made.len(), &mut [0u8; 1]).is_err());
        assert!(made.read_at(u64::MAX, &mut [0u8; 2]).is_err());
    }

    /// Counts the file's reads, and which began the blob.
    #[derive(Default)]
    struct Counted(Mutex<Vec<bool>>);

    impl FileReads for Counted {
        fn read(&self, begins: bool, read: &mut dyn FnMut() -> io::Result<u64>) -> io::Result<()> {
            self.0.lock().unwrap().push(begins);
            read().map(|_| ())
        }
    }

    #[test]
    fn only_a_chunk_not_cached_reads_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let plaintext = vec![9u8; 2 * CHUNK_SIZE];
        let path = dir.path().join("f");
        std::fs::write(&path, &plaintext).unwrap();
        let mut blob = Vec::new();
        let info = encrypt_blob(
            cipher().as_ref(),
            &mut io::Cursor::new(&plaintext),
            &mut blob,
        )
        .unwrap();
        let counted = Arc::new(Counted::default());
        let made = MadeBlob::open(
            &crate::sender_files::LocalFiles,
            &path,
            1,
            cipher(),
            plaintext.len() as u64,
            &info.plaintext_hash,
        )
        .unwrap()
        .with_reads(counted.clone());
        // 16 KiB groups, as bao-tree asks: four per chunk.
        let mut out = vec![0u8; 16 * 1024];
        let mut offset = 0;
        let mut made_bytes = Vec::new();
        while offset < blob.len() {
            let len = out.len().min(blob.len() - offset);
            made.read_at(offset as u64, &mut out[..len]).unwrap();
            made_bytes.extend_from_slice(&out[..len]);
            offset += len;
        }
        assert_eq!(made_bytes, blob);
        // One read per chunk, the first one beginning the blob.
        assert_eq!(*counted.0.lock().unwrap(), [true, false]);
    }
}
