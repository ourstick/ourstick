//! The owner's side of sharing a folder: estimate the cache, catalogue the
//! folder and fill the cache behind it, sign a record for each new manifest,
//! and publish. See ADR 0005 and ADR 0007.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::blob::blob_len;
use crate::cipher::{Cipher, CipherError, StickKey};
use crate::clock::Clock;
use crate::manifest::build::regular_files;
use crate::manifest::{Manifest, ManifestError, DRIVE_MARKER};
use crate::record::SignedManifestRecord;
use crate::stick_id::{StickId, StickSigningKey};
use crate::store::{Store, StoreError};
use crate::transport::{BlobHash, Publisher, TransportError};

mod files_keeper;
mod latest;
mod partial;
mod pass;
mod reports;
mod served;
mod version;

pub use files_keeper::{keep_files, nudge_later, PassWanted};
pub use latest::{load_latest_manifest, serve_latest};
pub use pass::{run_pass, PartialTiming, PassEvent, PassOptions, PassOutcome, PARTIAL_TIMING};
pub use reports::{has, keep_reports, serving_now, Has};
pub use served::served_files;

/// What the owner of a stick publishes with.
pub struct Owner<'a, P> {
    pub publisher: &'a P,
    pub store: Arc<dyn Store>,
    /// The disks OurStick reads, shared by every stick (ADR 0013).
    pub disks: Arc<crate::disk::turns::DiskTurns>,
    pub clock: &'a dyn Clock,
    pub signing_key: &'a StickSigningKey,
    pub stick_key: &'a StickKey,
}

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
pub(crate) fn outboard_len(blob_len: u64) -> u64 {
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
    #[error(transparent)]
    Space(#[from] crate::space::SpaceError),
    #[error("{} is not there (unplugged, moved or renamed?); readers wait until it is back", .name.as_ref().map_or_else(|| "the stick's folder".to_owned(), |name| format!("\u{201c}{name}\u{201d}")))]
    SourceGone { name: Option<String> },
    #[error("the stick's files already have a keeper")]
    KeeperRunning,
    /// A linked entry was added or removed while the pass read: it ends
    /// without recording or publishing more, and another pass reads the
    /// stick as it is now.
    #[error("the stick's folders changed while they were read; reading again")]
    LayoutChanged,
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

/// Runs a pass over the stick's folder and publishes its complete version
/// at the end (no partial versions): a new `seq` if the manifest changed,
/// otherwise the latest record again. See [`run_pass`].
///
/// The folder, cipher and last `seq` come from the owned stick in `store`.
/// A file whose size, mtime and ctime are as at the last pass is not read
/// again (a heuristic; see "Known limits" in `docs/protocol.md`); its blob is
/// encrypted again only if the cache no longer holds it. With `rescan`,
/// every file is read. An entry linked or removed while it reads starts it
/// again (what was recorded is not read twice), a few times at most.
pub async fn publish_folder(
    publisher: &impl Publisher,
    store: Arc<dyn Store>,
    clock: &dyn Clock,
    signing_key: &StickSigningKey,
    stick_key: &StickKey,
    rescan: bool,
) -> Result<Publication, ShareError> {
    // One pass alone: its own disks (a sharing program gives every stick
    // the same ones instead).
    let disks = Arc::new(crate::disk::turns::DiskTurns::new(Arc::new(
        crate::disk::turns::SystemTimer::default(),
    )));
    let owner = Owner {
        publisher,
        store,
        disks,
        clock,
        signing_key,
        stick_key,
    };
    let options = PassOptions {
        rescan,
        partial: None,
    };
    let mut tries = 1;
    loop {
        match run_pass(&owner, options, &mut |_| {}).await {
            Err(ShareError::LayoutChanged) if tries < LAYOUT_TRIES => tries += 1,
            passed => return Ok(passed?.publication),
        }
    }
}

/// How many passes [`publish_folder`] runs while the stick's entries keep
/// changing under it.
const LAYOUT_TRIES: u32 = 5;

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
    use std::io::Write;
    let path = root.join(DRIVE_MARKER);
    let io = |source| ShareError::Io {
        path: path.clone(),
        source,
    };
    let text = drive_marker_text(stick_id);
    // Written over in place, never emptied first, so a crash leaves the old
    // id or the new one, never nothing (every id has the same length). Not
    // made anew either: Windows refuses that for a hidden file, and the app
    // hides the marker there.
    let mut file = match std::fs::OpenOptions::new().write(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::File::create_new(&path).map_err(io)?
        }
        Err(error) => return Err(io(error)),
    };
    file.write_all(text.as_bytes()).map_err(io)?;
    file.set_len(text.len() as u64).map_err(io)?;
    file.sync_all().map_err(io)
}

/// What the drive marker of `stick_id` holds: the StickId in hex.
fn drive_marker_text(stick_id: &StickId) -> String {
    data_encoding::HEXLOWER.encode(&stick_id.to_bytes())
}

/// Whether `root` holds the drive marker of `stick_id` (the drive is
/// plugged in). Blocking.
pub(crate) fn drive_present(root: &Path, stick_id: &StickId) -> bool {
    drive_marked_by(root, stick_id)
}

/// Whether `root` is the drive of `stick_id`: its marker is there and names
/// this stick (an app looking for a stick's folder). Blocking.
pub fn drive_marked_by(root: &Path, stick_id: &StickId) -> bool {
    let marker = std::fs::read_to_string(root.join(DRIVE_MARKER)).unwrap_or_default();
    marker.trim() == drive_marker_text(stick_id)
}

/// Checks that `root` is the drive of `stick_id`: its marker is there and
/// names this stick. Blocking.
fn check_drive(root: &Path, stick_id: &StickId) -> Result<(), ShareError> {
    if drive_present(root, stick_id) {
        Ok(())
    } else {
        Err(ShareError::DriveMissing {
            path: root.to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drive_is_known_by_its_marker_alone() {
        let dir = tempfile::tempdir().unwrap();
        let stick = crate::stick_id::StickSigningKey::from_bytes(&[1; 32]).stick_id();
        let other = crate::stick_id::StickSigningKey::from_bytes(&[2; 32]).stick_id();
        assert!(!drive_marked_by(dir.path(), &stick));
        mark_drive(dir.path(), &stick).unwrap();
        assert!(drive_marked_by(dir.path(), &stick));
        assert!(!drive_marked_by(dir.path(), &other));
        // Marked again for another stick, in place.
        mark_drive(dir.path(), &other).unwrap();
        assert!(drive_marked_by(dir.path(), &other));
        assert!(!drive_marked_by(dir.path(), &stick));
    }
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
}
