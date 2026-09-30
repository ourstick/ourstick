//! The owner's side of sharing a folder: estimate the cache, encrypt the folder
//! into it, sign a record for the new manifest, and publish. See ADR 0005.

use std::collections::{BTreeSet, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::blob::{blob_len, encrypt_blob};
use crate::blocking::run_blocking;
use crate::cipher::{cipher_for, Cipher, CipherError, StickKey};
use crate::clock::Clock;
use crate::manifest::build::regular_files;
use crate::manifest::{Encrypted, KnownFiles, Manifest, ManifestError, DRIVE_MARKER};
use crate::record::{next_seq, SignedManifestRecord};
use crate::stick_id::{StickId, StickSigningKey};
use crate::store::{OwnedFile, Store, StoreError};
use crate::transport::{BlobHash, Publisher, TransportError};

/// What sharing a folder will take, known before anything is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharePlan {
    pub files: u64,
    pub plaintext_bytes: u64,
    /// Estimated size of the encrypted cache: every blob plus its bao outboard.
    pub cache_bytes: u64,
}

/// Size of the bao outboard iroh-blobs keeps for a blob: 64 bytes per pair of
/// 16 KiB chunk groups.
fn outboard_len(blob_len: u64) -> u64 {
    blob_len.div_ceil(16 * 1024).saturating_sub(1) * 64
}

/// Walks `root` (file sizes only, nothing is read) and estimates the cache.
/// Blocking: call from `spawn_blocking` in async code.
pub fn plan_share(root: &Path, cipher: &dyn Cipher) -> Result<SharePlan, ShareError> {
    let mut plan = SharePlan {
        files: 0,
        plaintext_bytes: 0,
        cache_bytes: 0,
    };
    for path in regular_files(root)? {
        let size = std::fs::metadata(&path)
            .map_err(|source| ShareError::Io {
                path: path.clone(),
                source,
            })?
            .len();
        let blob = blob_len(cipher, size);
        plan.files += 1;
        plan.plaintext_bytes += size;
        plan.cache_bytes += blob + outboard_len(blob);
    }
    Ok(plan)
}

/// A published version of a stick.
#[derive(Debug, Clone)]
pub struct Publication {
    pub manifest: Manifest,
    pub manifest_hash: BlobHash,
    /// The `seq` of `record`, for the ticket's manifest hint.
    pub seq: u64,
    pub record: SignedManifestRecord,
    /// False if the folder was as last published: the same record is served
    /// again, with no new `seq`.
    pub new_version: bool,
    /// Files that kept their blob from the last publish without being read.
    pub reused: usize,
}

/// Why sharing failed. The messages are meant for users.
#[derive(Debug, thiserror::Error)]
pub enum ShareError {
    #[error("this device does not own that stick")]
    NotOwned,
    #[error("the stick has signed so many versions that no seq is left")]
    SeqExhausted,
    #[error("the cache returned a different hash than expected for {0}")]
    CacheMismatch(PathBuf),
    #[error("the stick is not plugged in: {} is not its drive (ejected?)", .path.display())]
    DriveMissing { path: PathBuf },
    #[error("the stick is full: it holds {used} bytes, more than its {size}")]
    StickFull { size: u64, used: u64 },
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Cipher(#[from] CipherError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Removes leftovers of an earlier, interrupted run from the staging directory.
fn clear_staging(staging: &Path) -> Result<(), ShareError> {
    let io = |source| ShareError::Io {
        path: staging.to_path_buf(),
        source,
    };
    for entry in std::fs::read_dir(staging).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if path.is_file() {
            std::fs::remove_file(&path).map_err(|source| ShareError::Io { path, source })?;
        }
    }
    Ok(())
}

/// Encrypts the stick's folder into the publisher's cache, signs a record for
/// the new manifest with the next `seq`, stores it, and publishes it. If the
/// manifest is the one last signed, that record is published again instead:
/// a version changes only when the folder does.
///
/// The folder, cipher and last `seq` come from the owned stick in `store`.
///
/// A file whose size, mtime and ctime are as at the last publish, and whose
/// blob is still in the cache, is not encrypted again (a heuristic; see
/// "Known limits" in `docs/protocol.md`). With `rescan`, every file is.
pub async fn publish_folder(
    publisher: &impl Publisher,
    store: Arc<dyn Store>,
    clock: &dyn Clock,
    signing_key: &StickSigningKey,
    stick_key: &StickKey,
    rescan: bool,
) -> Result<Publication, ShareError> {
    let stick_id = signing_key.stick_id();
    let owned = {
        let store = store.clone();
        run_blocking(move || store.owned_stick(&stick_id)).await?
    }
    .ok_or(ShareError::NotOwned)?;
    let cipher = cipher_for(owned.cipher, stick_key);
    let staging = publisher.staging_dir().to_path_buf();
    if let Some(size) = owned.drive_size {
        // Never publish an ejected drive (an empty mount point) as an empty
        // stick, nor a stick that does not fit.
        let root = owned.root_path.clone();
        let used = run_blocking(move || check_drive(&root, &stick_id)).await?;
        if used > size {
            return Err(ShareError::StickFull { size, used });
        }
    }

    let known = if rescan {
        KnownFiles::new()
    } else {
        known_files(publisher, &store, &stick_id).await?
    };
    let (encrypted, blob_files, manifest_file, manifest_hash) = {
        let cipher = cipher.clone();
        let root = owned.root_path.clone();
        run_blocking(move || encrypt_stick(&root, cipher.as_ref(), &staging, &known)).await?
    };
    let Encrypted {
        manifest,
        files,
        reused,
        ..
    } = encrypted;

    for blob_file in blob_files.iter().chain([&manifest_file]) {
        let expected = blob_file
            .file_stem()
            .and_then(|stem| data_encoding::HEXLOWER.decode(stem.as_encoded_bytes()).ok());
        let hash = publisher.import_blob(blob_file).await?;
        if expected.as_deref() != Some(&hash[..]) {
            return Err(ShareError::CacheMismatch(blob_file.clone()));
        }
        tokio::fs::remove_file(blob_file)
            .await
            .map_err(|source| ShareError::Io {
                path: blob_file.clone(),
                source,
            })?;
    }

    let unchanged = owned
        .latest_record
        .filter(|record| record.unverified_record().manifest_hash == manifest_hash);
    let new_version = unchanged.is_none();
    let record = match unchanged {
        Some(record) => record,
        None => {
            let seq =
                next_seq(owned.last_signed_seq, clock.now_ms()).ok_or(ShareError::SeqExhausted)?;
            let record = signing_key.sign_record(seq, manifest_hash);
            let store = store.clone();
            let saved = record.clone();
            // Stored before it is served, so a seq is never used twice.
            run_blocking(move || store.save_signed_record(&saved, false)).await?;
            record
        }
    };
    let seq = record.unverified_record().seq;

    let mut allowed: HashSet<BlobHash> = manifest.entries.iter().map(|entry| entry.hash).collect();
    allowed.insert(manifest_hash);
    publisher.publish(record.clone(), allowed.clone()).await?;
    publisher.keep_only_blobs(allowed).await?;
    {
        let files: Vec<OwnedFile> = files
            .into_iter()
            .map(|(path, stamp, blob_hash)| OwnedFile {
                path,
                stamp,
                blob_hash,
                plaintext_hash: None,
            })
            .collect();
        let store = store.clone();
        run_blocking(move || {
            let paths: Vec<String> = files.iter().map(|file| file.path.clone()).collect();
            store.record_owned_files(&stick_id, &files)?;
            store.keep_only_owned_files(&stick_id, &paths)
        })
        .await?;
    }
    tracing::info!(
        seq,
        new_version,
        reused,
        files = manifest.entries.len(),
        "published stick"
    );
    Ok(Publication {
        manifest,
        manifest_hash,
        seq,
        record,
        new_version,
        reused,
    })
}

/// The files recorded at the last publish whose blobs the cache still holds;
/// each of those blobs is kept from now on, as if imported.
async fn known_files(
    publisher: &impl Publisher,
    store: &Arc<dyn Store>,
    stick_id: &StickId,
) -> Result<KnownFiles, ShareError> {
    let recorded = {
        let store = store.clone();
        let stick_id = *stick_id;
        run_blocking(move || store.owned_files(&stick_id)).await?
    };
    let mut known = KnownFiles::new();
    for file in recorded {
        if publisher.keep_blob(&file.blob_hash).await? {
            known.insert(file.path, (file.stamp, file.blob_hash));
        }
    }
    Ok(known)
}

/// The latest record this device signed for `stick_id`, as a ticket's
/// manifest hint; `None` before the first publish. Blocking.
pub fn latest_hint(
    store: &dyn Store,
    stick_id: &StickId,
) -> Result<Option<crate::record::ManifestHint>, ShareError> {
    let stick = store.owned_stick(stick_id)?.ok_or(ShareError::NotOwned)?;
    Ok(stick.latest_record.map(|record| {
        let record = record.unverified_record();
        crate::record::ManifestHint {
            seq: record.seq,
            hash: record.manifest_hash,
        }
    }))
}

/// The bytes in the files of `root` that belong to the stick (system files
/// left out, see [`crate::manifest::ignore`]). Blocking.
pub fn folder_size(root: &Path) -> Result<u64, ShareError> {
    let mut size = 0;
    for path in regular_files(root)? {
        let metadata = std::fs::metadata(&path).map_err(|source| ShareError::Io {
            path: path.clone(),
            source,
        })?;
        size += metadata.len();
    }
    Ok(size)
}

/// Makes `root` the drive of `stick_id` by writing the drive marker into it.
/// Blocking.
pub fn mark_drive(root: &Path, stick_id: &StickId) -> Result<(), ShareError> {
    let path = root.join(DRIVE_MARKER);
    std::fs::write(&path, drive_marker_text(stick_id))
        .map_err(|source| ShareError::Io { path, source })
}

/// What the drive marker of `stick_id` holds: the StickId in hex.
fn drive_marker_text(stick_id: &StickId) -> String {
    data_encoding::HEXLOWER.encode(&stick_id.to_bytes())
}

/// Checks that `root` is the drive of `stick_id` (its marker is there and
/// names this stick) and returns its size. Blocking.
fn check_drive(root: &Path, stick_id: &StickId) -> Result<u64, ShareError> {
    let marker = std::fs::read_to_string(root.join(DRIVE_MARKER)).unwrap_or_default();
    if marker.trim() != drive_marker_text(stick_id) {
        return Err(ShareError::DriveMissing {
            path: root.to_path_buf(),
        });
    }
    folder_size(root)
}

/// Blocking part of [`publish_folder`]: encrypts every file not in `known`,
/// and the manifest, into `staging`, each as `<hex hash>.blob`.
fn encrypt_stick(
    root: &Path,
    cipher: &dyn Cipher,
    staging: &Path,
    known: &KnownFiles,
) -> Result<(Encrypted, Vec<PathBuf>, PathBuf, BlobHash), ShareError> {
    clear_staging(staging)?;
    let encrypted = Manifest::from_dir_reusing(root, cipher, staging, known)?;
    let manifest = &encrypted.manifest;
    // Identical files share one blob; import each blob once.
    let blob_files: Vec<PathBuf> = encrypted
        .new_blobs
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut manifest_blob = Vec::new();
    encrypt_blob(
        cipher,
        &mut Cursor::new(manifest.to_bytes()?),
        &mut manifest_blob,
    )?;
    let manifest_hash = *blake3::hash(&manifest_blob).as_bytes();
    let manifest_file = staging.join(format!(
        "{}.blob",
        data_encoding::HEXLOWER.encode(&manifest_hash)
    ));
    std::fs::write(&manifest_file, &manifest_blob).map_err(|source| ShareError::Io {
        path: manifest_file.clone(),
        source,
    })?;
    Ok((encrypted, blob_files, manifest_file, manifest_hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::{ChaChaCipher, NoCipher};

    #[test]
    fn plan_counts_files_and_estimates_cache() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("a"), vec![1u8; 100]).unwrap();
        std::fs::write(dir.path().join("sub/b"), vec![2u8; 200_000]).unwrap();

        let cipher = ChaChaCipher::new(&StickKey::from_bytes([1; 32]));
        let plan = plan_share(dir.path(), &cipher).unwrap();
        assert_eq!(plan.files, 2);
        assert_eq!(plan.plaintext_bytes, 200_100);
        let blobs = blob_len(&cipher, 100) + blob_len(&cipher, 200_000);
        assert!(plan.cache_bytes > blobs && plan.cache_bytes < blobs + 2_000);

        let plain = plan_share(dir.path(), &NoCipher).unwrap();
        assert!(plain.cache_bytes < plan.cache_bytes);
    }

    #[test]
    fn outboard_len_matches_bao_for_16_kib_groups() {
        assert_eq!(outboard_len(0), 0);
        assert_eq!(outboard_len(16 * 1024), 0);
        assert_eq!(outboard_len(16 * 1024 + 1), 64);
        assert_eq!(outboard_len(1024 * 1024), 63 * 64);
    }

    #[test]
    fn encrypt_stick_names_every_blob_after_its_hash() {
        let dir = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"same").unwrap();
        std::fs::write(dir.path().join("b"), b"same").unwrap();
        std::fs::write(staging.path().join("leftover.partial"), b"x").unwrap();
        let cipher = ChaChaCipher::new(&StickKey::from_bytes([1; 32]));

        let (encrypted, blobs, manifest_file, manifest_hash) =
            encrypt_stick(dir.path(), &cipher, staging.path(), &KnownFiles::new()).unwrap();
        assert_eq!(encrypted.manifest.entries.len(), 2);
        assert_eq!(blobs.len(), 1, "identical files share one blob");
        let bytes = std::fs::read(&manifest_file).unwrap();
        assert_eq!(*blake3::hash(&bytes).as_bytes(), manifest_hash);
        assert!(!staging.path().join("leftover.partial").exists());
    }
}
