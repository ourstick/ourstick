//! Hashing one file into its blob's hash (ADR 0005) and, for the owner's
//! catalogue, the blob's hash tree (ADR 0017, rule 1): the bao outboard
//! that lets each 16 KiB group be checked when the blob is made again.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

use bao_tree::io::outboard::PreOrderOutboard;
use bao_tree::{BaoTree, BlockSize};

use super::build::relative_path;
use super::{Entry, ManifestError};
use crate::blob::{blob_len, cipher_error, BlobReader};
use crate::cipher::{Cipher, CipherError};
use crate::share::disk_turns::DiskReader;

/// iroh's chunk groups: 16 KiB (`iroh_blobs::store::IROH_BLOCK_SIZE`).
pub(crate) const BLOCK_SIZE: BlockSize = BlockSize::from_chunk_log(4);

/// Encrypts one file, hashes the resulting blob and builds the entry. The size is
/// the number of plaintext bytes actually encrypted, so it always matches the hash.
pub(crate) fn hash_file(
    root: &Path,
    path: &Path,
    cipher: &dyn Cipher,
) -> Result<Entry, ManifestError> {
    hash_file_telling_plaintext(relative_path(root, path)?, path, cipher, None, None)
        .map(|hashed| hashed.entry)
}

/// What hashing a file for the catalogue gives.
#[derive(Debug)]
pub(crate) struct Hashed {
    pub(crate) entry: Entry,
    /// The plaintext hash (the nonce prefix's source, ADR 0003).
    pub(crate) plaintext_hash: [u8; 32],
}

/// [`hash_file`] for the file at `path` whose manifest path is `relative`,
/// also telling the plaintext hash. With `reader`, the file is read through
/// its disk's turns (ADR 0013). With `outboards`, a blob of more than one
/// 16 KiB group has its outboard written there as `<hex hash>.obao`
/// (through a temporary file and a rename); a smaller one has none. Blocking.
pub(crate) fn hash_file_telling_plaintext(
    relative: String,
    path: &Path,
    cipher: &dyn Cipher,
    reader: Option<&mut DiskReader>,
    outboards: Option<&Path>,
) -> Result<Hashed, ManifestError> {
    let file = File::open(path).map_err(|e| ManifestError::io(path, e))?;
    let metadata = file.metadata().map_err(|e| ManifestError::io(path, e))?;
    let hashed = match reader {
        Some(reader) => {
            let len = metadata.len();
            hash_plaintext(
                cipher,
                &mut BufReader::new(reader.open(file, len)),
                outboards,
            )
        }
        None => hash_plaintext(cipher, &mut BufReader::new(file), outboards),
    }
    .map_err(|error| match error {
        CipherError::Io(source) => ManifestError::io(path, source),
        source => ManifestError::Blob {
            path: path.to_path_buf(),
            source,
        },
    })?;
    let mtime = metadata
        .modified()
        .map_err(|e| ManifestError::io(path, e))?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
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
/// [`BlobReader`] into the blob's hash and, if it has more than one group,
/// its outboard. The tree's size comes from the first read; if the second
/// gives other bytes, more or fewer, it is [`CipherError::PlaintextChanged`]
/// and no outboard is kept.
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
    let mut blob = BlobReader::new(cipher, plaintext, plaintext_hash)?;
    let mut outboard = PreOrderOutboard {
        root: blake3::Hash::from_bytes([0; 32]),
        tree,
        data: Vec::new(),
    };
    let root = bao_tree::io::sync::outboard(&mut blob, tree, &mut outboard).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            CipherError::PlaintextChanged
        } else {
            cipher_error(error)
        }
    })?;
    // The blob must end where the tree does: a byte more is a changed file,
    // and reading to the end is what checks the plaintext's hash.
    if blob.read(&mut [0u8; 1]).map_err(cipher_error)? != 0 {
        return Err(CipherError::PlaintextChanged);
    }
    if blob.info().is_none() {
        return Err(CipherError::PlaintextChanged);
    }
    let blob_hash = *root.as_bytes();
    if let Some(dir) = outboards {
        if !outboard.data.is_empty() {
            keep_outboard(dir, &blob_hash, &outboard.data)?;
        }
    }
    Ok(Hashes {
        plaintext_len,
        plaintext_hash: *plaintext_hash.as_bytes(),
        blob_hash,
    })
}

/// The folder of a stick's outboards, in its cache folder.
pub(crate) fn outboards_dir(cache_dir: &Path) -> std::path::PathBuf {
    cache_dir.join("outboards")
}

/// Removes from `dir` the outboards of blobs not in `keep`, and what a write
/// cut short left. A missing folder is nothing to do. Blocking.
pub(crate) fn keep_only_outboards(
    dir: &Path,
    keep: &std::collections::HashSet<[u8; 32]>,
) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let kept = name
            .to_str()
            .and_then(|name| name.strip_suffix(".obao"))
            .and_then(|hex| data_encoding::HEXLOWER.decode(hex.as_bytes()).ok())
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .is_some_and(|hash| keep.contains(&hash));
        if !kept {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Where the outboard of `blob_hash` is kept in `dir`.
pub(crate) fn outboard_path(dir: &Path, blob_hash: &[u8; 32]) -> std::path::PathBuf {
    dir.join(format!(
        "{}.obao",
        data_encoding::HEXLOWER.encode(blob_hash)
    ))
}

fn keep_outboard(dir: &Path, blob_hash: &[u8; 32], data: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let target = outboard_path(dir, blob_hash);
    let temporary = target.with_extension("obao.partial");
    std::fs::write(&temporary, data)?;
    std::fs::rename(&temporary, &target)
}

#[cfg(test)]
#[path = "hashing_tests.rs"]
mod tests;
