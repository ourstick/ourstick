//! Hashing one file into its blob's hash (ADR 0005) and, for the owner's
//! catalogue, the blob's hash tree (ADR 0017, rule 1): the bao outboard
//! that lets each 16 KiB group be checked when the blob is made again.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use bao_tree::io::outboard::PreOrderOutboard;
use bao_tree::{BaoTree, BlockSize};

use super::build::relative_path;
use super::{Entry, ManifestError};
use crate::blob::{blob_len, cipher_error, BlobReader};
use crate::cipher::{Cipher, CipherError};
use crate::disk::turns::DiskReader;
use crate::sender_files::{InOrder, LocalFiles, SenderFiles};

/// iroh's chunk groups (16 KiB): the outboards must be the ones iroh-blobs
/// serves with.
pub(crate) const BLOCK_SIZE: BlockSize = iroh_blobs::store::IROH_BLOCK_SIZE;

/// How much of a file is read at once when it is hashed: a phone reads
/// each piece through Kotlin, so few large pieces (ADR 0029).
const READ_SIZE: usize = 256 << 10;

/// Encrypts one file, hashes the resulting blob and builds the entry. The size is
/// the number of plaintext bytes actually encrypted, so it always matches the hash.
pub(crate) fn hash_file(
    root: &Path,
    path: &Path,
    cipher: &dyn Cipher,
) -> Result<Entry, ManifestError> {
    hash_file_telling_plaintext(
        &LocalFiles,
        relative_path(root, path)?,
        path,
        cipher,
        None,
        None,
    )
    .map(|hashed| hashed.entry)
}

/// What hashing a file for the catalogue gives.
#[derive(Debug)]
pub(crate) struct Hashed {
    pub(crate) entry: Entry,
    /// The plaintext hash (the nonce prefix's source, ADR 0003).
    pub(crate) plaintext_hash: [u8; 32],
}

/// [`hash_file`] for the file at `path`, read through `files`, whose
/// manifest path is `relative`, also telling the plaintext hash. With `reader`, the file is read through
/// its disk's turns (ADR 0013). With `outboards`, a blob of more than one
/// 16 KiB group has its outboard written there as `<hex hash>.obao`
/// (through a temporary file and a rename); a smaller one has none. Blocking.
pub(crate) fn hash_file_telling_plaintext(
    files: &dyn SenderFiles,
    relative: String,
    path: &Path,
    cipher: &dyn Cipher,
    reader: Option<&mut DiskReader>,
    outboards: Option<&Path>,
) -> Result<Hashed, ManifestError> {
    let file = files.open(path).map_err(|e| ManifestError::io(path, e))?;
    let facts = file.facts().map_err(|e| ManifestError::io(path, e))?;
    let file = InOrder::new(file);
    let hashed = match reader {
        Some(reader) => {
            let len = facts.stamp.size;
            hash_plaintext(
                cipher,
                &mut BufReader::with_capacity(READ_SIZE, reader.open(file, len)),
                outboards,
            )
        }
        None => hash_plaintext(
            cipher,
            &mut BufReader::with_capacity(READ_SIZE, file),
            outboards,
        ),
    }
    .map_err(|error| match error {
        CipherError::Io(source) => ManifestError::io(path, source),
        source => ManifestError::Blob {
            path: path.to_path_buf(),
            source,
        },
    })?;
    let mtime = facts.mtime_seconds();
    Ok(Hashed {
        entry: Entry {
            path: relative,
            size: hashed.plaintext_len,
            mtime,
            hash: hashed.blob_hash,
        },
        plaintext_hash: hashed.plaintext_hash,
    })
}

struct Hashes {
    plaintext_len: u64,
    plaintext_hash: [u8; 32],
    blob_hash: [u8; 32],
}

/// Reads `plaintext` twice: to hash it (the nonce prefix), then through a
/// [`BlobReader`] into the blob's hash and, with `outboards` and more than
/// one group, its outboard, written as it is made (never held in memory: a
/// 100 GB file has a 400 MB one). The tree's size comes from the first read;
/// if the second gives other bytes, more or fewer, it is
/// [`CipherError::PlaintextChanged`] and no outboard is kept. A read that
/// fails is that I/O error, not a change.
fn hash_plaintext<R: Read + Seek>(
    cipher: &dyn Cipher,
    plaintext: &mut R,
    outboards: Option<&Path>,
) -> Result<Hashes, CipherError> {
    let start = plaintext.stream_position()?;
    let mut first_pass = blake3::Hasher::new();
    first_pass.update_reader(&mut *plaintext)?;
    let plaintext_hash = first_pass.finalize();
    let plaintext_len = first_pass.count();
    plaintext.seek(SeekFrom::Start(start))?;

    let tree = BaoTree::new(blob_len(cipher, plaintext_len), BLOCK_SIZE);
    let mut blob = Ends {
        blob: BlobReader::new(cipher, plaintext, plaintext_hash)?,
        ended: false,
    };
    let one_group = tree.size() <= BLOCK_SIZE.bytes() as u64;
    let blob_hash = match outboards {
        Some(dir) if !one_group => hash_keeping_outboard(&mut blob, tree, dir)?,
        // The root of a blob is its BLAKE3 hash, whatever its groups.
        _ => {
            let mut hasher = blake3::Hasher::new();
            std::io::copy(&mut blob, &mut hasher).map_err(cipher_error)?;
            if hasher.count() != tree.size() {
                return Err(CipherError::PlaintextChanged);
            }
            *hasher.finalize().as_bytes()
        }
    };
    Ok(Hashes {
        plaintext_len,
        plaintext_hash: *plaintext_hash.as_bytes(),
        blob_hash,
    })
}

/// A [`BlobReader`] that tells whether it ended, so that a blob shorter than
/// its tree (a changed file) is told apart from a read that failed.
struct Ends<'a, R: Read> {
    blob: BlobReader<'a, R>,
    ended: bool,
}

impl<R: Read> Read for Ends<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.blob.read(out)?;
        if n == 0 && !out.is_empty() {
            self.ended = true;
        }
        Ok(n)
    }
}

/// The blob's hash, its outboard written to `dir` through a temporary file
/// of a name of its own, renamed once the blob is read to its end and
/// checked.
fn hash_keeping_outboard<R: Read>(
    blob: &mut Ends<'_, R>,
    tree: BaoTree,
    dir: &Path,
) -> Result<[u8; 32], CipherError> {
    std::fs::create_dir_all(dir)?;
    let temporary = dir.join(format!(".{}.obao.partial", unique_suffix()));
    let result = (|| {
        let file = File::create_new(&temporary)?;
        let mut outboard = PreOrderOutboard {
            root: blake3::Hash::from_bytes([0; 32]),
            tree,
            data: file,
        };
        let root =
            bao_tree::io::sync::outboard(&mut *blob, tree, &mut outboard).map_err(|error| {
                if blob.ended {
                    CipherError::PlaintextChanged
                } else {
                    cipher_error(error)
                }
            })?;
        // The blob must end where the tree does: a byte more is a changed
        // file, and reading to the end is what checks the plaintext's hash.
        if blob.read(&mut [0u8; 1]).map_err(cipher_error)? != 0 {
            return Err(CipherError::PlaintextChanged);
        }
        let blob_hash = *root.as_bytes();
        drop(outboard);
        let target = outboard_path(dir, &blob_hash);
        // The one there is kept if it is this one (on Windows it may be
        // open for a transfer, which a rename over it could fail on); one
        // cut short by a crash, or damaged, is replaced. An outboard is 0.4 %
        // of its blob: reading it again costs little.
        let same = std::fs::metadata(&target)
            .is_ok_and(|metadata| metadata.len() == tree.outboard_size())
            && same_bytes(&target, &temporary);
        if same {
            std::fs::remove_file(&temporary)?;
        } else {
            std::fs::rename(&temporary, &target)?;
        }
        Ok(blob_hash)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Whether two files hold the same bytes, read a piece at a time; false if
/// either cannot be read.
fn same_bytes(one: &Path, other: &Path) -> bool {
    let compare = || -> std::io::Result<bool> {
        let mut one = std::io::BufReader::new(File::open(one)?);
        let mut other = std::io::BufReader::new(File::open(other)?);
        let mut piece = vec![0u8; 64 * 1024];
        let mut other_piece = vec![0u8; 64 * 1024];
        loop {
            let read = one.read(&mut piece)?;
            if read == 0 {
                return Ok(other.read(&mut other_piece[..1])? == 0);
            }
            other.read_exact(&mut other_piece[..read])?;
            if piece[..read] != other_piece[..read] {
                return Ok(false);
            }
        }
    };
    compare().unwrap_or(false)
}

/// A random suffix for temporary file names.
fn unique_suffix() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    data_encoding::HEXLOWER.encode(&bytes)
}

/// The folder of a stick's outboards, in its cache folder.
pub(crate) fn outboards_dir(cache_dir: &Path) -> std::path::PathBuf {
    cache_dir.join("outboards")
}

/// Removes from `dir` the outboards of blobs not in `keep`, and what a write
/// cut short left (one pass of a stick runs at a time, so no write is under
/// way). Goes through every file even if one cannot be removed, and returns
/// the first such error. A missing folder is nothing to do. Blocking.
pub(crate) fn keep_only_outboards(
    dir: &Path,
    keep: &std::collections::HashSet<[u8; 32]>,
) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut first_error = None;
    for entry in entries {
        let removed = entry.and_then(|entry| {
            let kept = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".obao"))
                .and_then(|hex| data_encoding::HEXLOWER.decode(hex.as_bytes()).ok())
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                .is_some_and(|hash| keep.contains(&hash));
            if kept {
                return Ok(());
            }
            match std::fs::remove_file(entry.path()) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        });
        if let Err(error) = removed {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Where the outboard of `blob_hash` is kept in `dir`.
pub(crate) fn outboard_path(dir: &Path, blob_hash: &[u8; 32]) -> std::path::PathBuf {
    dir.join(format!(
        "{}.obao",
        data_encoding::HEXLOWER.encode(blob_hash)
    ))
}

#[cfg(test)]
#[path = "hashing_tests.rs"]
mod tests;
