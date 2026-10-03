//! Preparing one suitcase (ADR 0007): each file is encrypted again into the
//! suitcase's folder, reading it once when the catalogue's plaintext hash
//! belongs to this very version of the file, and its blob is served only if
//! its hash is the manifest's. A file that changed since the catalogue is
//! left out, and what was written for it is deleted at once.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, BufWriter, ErrorKind, Write};
use std::path::PathBuf;
use std::sync::Arc;

use super::super::disk_turns::{DiskReader, Disks, StopOnDrop};
use crate::blob::{encrypt_blob, encrypt_blob_hashed_before};
use crate::blocking::run_blocking;
use crate::cipher::{Cipher, CipherError};
use crate::layout::Root;
use crate::manifest::Entry;
use crate::share::ShareError;
use crate::transport::{BlobHash, Publisher, SuitcaseId};

/// A reader told "not ready" this recently is still waiting (its pauses
/// grow to a minute): a suitcase for it goes first on the disk.
const READERS_WAIT_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// How a suitcase's files are read: through their disks' turns.
pub(super) struct Reading {
    pub(super) disks: Arc<Disks>,
    pub(super) urgent: bool,
    pub(super) stopped: Arc<std::sync::atomic::AtomicBool>,
}

/// One file to put in a suitcase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileJob {
    pub(super) entry: Entry,
    pub(super) on_disk: PathBuf,
    /// The root it is under: if that is gone, the file is not "changed"
    /// (its record stays) but out of reach for now.
    pub(super) root: Root,
    /// The catalogue's plaintext hash of this version of the file (same
    /// blob hash), if it has one.
    pub(super) plaintext_hash: Option<[u8; 32]>,
}

/// A prepared suitcase.
#[derive(Debug)]
pub(super) struct Prepared {
    pub(super) id: SuitcaseId,
    /// The blobs it serves.
    pub(super) ready: HashSet<BlobHash>,
    /// Files that changed since the catalogue (left out), by path.
    pub(super) changed: Vec<String>,
}

/// Opens a suitcase and puts the blobs of `files` in it, each ready as soon
/// as it is in.
pub(super) async fn prepare(
    publisher: &impl Publisher,
    cipher: &Arc<dyn Cipher>,
    disks: &Arc<Disks>,
    files: Vec<FileJob>,
) -> Result<Prepared, ShareError> {
    let (id, folder) = publisher.open_suitcase().await?;
    match fill(publisher, cipher, disks, files, id, &folder).await {
        Ok(prepared) => Ok(prepared),
        Err(error) => {
            // Half a suitcase is not kept: it is prepared again later.
            if let Err(evicted) = publisher.evict(id).await {
                tracing::warn!(error = %evicted, "a suitcase could not be removed");
            }
            Err(error)
        }
    }
}

async fn fill(
    publisher: &impl Publisher,
    cipher: &Arc<dyn Cipher>,
    disks: &Arc<Disks>,
    files: Vec<FileJob>,
    id: SuitcaseId,
    folder: &std::path::Path,
) -> Result<Prepared, ShareError> {
    // A suitcase dropped (its stick stopped) stops waiting for the disk.
    let stop_on_drop = StopOnDrop::default();
    let mut prepared = Prepared {
        id,
        ready: HashSet::new(),
        changed: Vec::new(),
    };
    for (number, file) in files.into_iter().enumerate() {
        let hash = file.entry.hash;
        let path = file.entry.path.clone();
        if prepared.ready.contains(&hash) {
            continue;
        }
        let blob_file = folder.join(format!("{number}.blob"));
        let written = {
            let cipher = cipher.clone();
            let blob_file = blob_file.clone();
            // Readers waiting for it: it goes first on the disk (ADR 0013).
            let urgent = publisher.readers_waiting(READERS_WAIT_WINDOW);
            let reading = Reading {
                disks: disks.clone(),
                urgent,
                stopped: stop_on_drop.flag(),
            };
            run_blocking(move || write_blob(cipher.as_ref(), &file, &blob_file, &reading)).await?
        };
        if !written {
            prepared.changed.push(path);
            continue;
        }
        if publisher.add_to_suitcase(id, &blob_file, &hash).await? {
            prepared.ready.insert(hash);
        } else {
            prepared.changed.push(path);
        }
    }
    Ok(prepared)
}

/// Encrypts `file` into `blob_file`. False if the file is gone or changed
/// since the catalogue: then nothing is left in `blob_file`. If its root is
/// gone, [`ShareError::SourceGone`]. Blocking.
fn write_blob(
    cipher: &dyn Cipher,
    file: &FileJob,
    blob_file: &std::path::Path,
    reading: &Reading,
) -> Result<bool, ShareError> {
    let io = |path: &std::path::Path| {
        let path = path.to_path_buf();
        move |source| ShareError::Io { path, source }
    };
    let gone = || ShareError::SourceGone {
        name: file.root.name.clone(),
    };
    let plaintext = match File::open(&file.on_disk) {
        Ok(plaintext) => plaintext,
        Err(_) if file.root.is_gone() => return Err(gone()),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io(&file.on_disk)(error)),
    };
    let len = plaintext.metadata().map_err(io(&file.on_disk))?.len();
    let stopped = reading.stopped.clone();
    let mut reader = DiskReader::new(reading.disks.of(&file.on_disk), reading.urgent, move || {
        if stopped.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(std::io::Error::other("the stick stopped"));
        }
        Ok(())
    });
    let mut plaintext = BufReader::new(reader.open(plaintext, len));
    let mut out = BufWriter::new(File::create_new(blob_file).map_err(io(blob_file))?);
    let encrypted = match &file.plaintext_hash {
        Some(hash) => encrypt_blob_hashed_before(cipher, &mut plaintext, &mut out, hash),
        None => encrypt_blob(cipher, &mut plaintext, &mut out),
    }
    .and_then(|info| {
        out.flush()?;
        Ok(info)
    });
    match encrypted {
        Ok(info) if info.plaintext_len == file.entry.size => Ok(true),
        Ok(_) | Err(CipherError::PlaintextChanged) => {
            // Its nonce may belong to other bytes (ADR 0003): gone at once.
            drop(out);
            std::fs::remove_file(blob_file).map_err(io(blob_file))?;
            Ok(false)
        }
        Err(CipherError::Io(source)) => {
            drop(out);
            let _ = std::fs::remove_file(blob_file);
            if file.root.is_gone() {
                return Err(gone());
            }
            Err(io(&file.on_disk)(source))
        }
        Err(error) => {
            drop(out);
            let _ = std::fs::remove_file(blob_file);
            Err(error.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cipher::{ChaChaCipher, StickKey};

    fn reading() -> Reading {
        Reading {
            disks: Arc::new(Disks::new(Arc::new(
                super::super::super::disk_turns::SystemTimer::default(),
            ))),
            urgent: false,
            stopped: Arc::default(),
        }
    }

    fn cipher() -> ChaChaCipher {
        ChaChaCipher::new(&StickKey::from_bytes([4; 32]))
    }

    fn job(dir: &std::path::Path, contents: &[u8], hashed: Option<&[u8]>) -> FileJob {
        let on_disk = dir.join("file");
        std::fs::write(&on_disk, contents).unwrap();
        let mut blob = Vec::new();
        let hashed = hashed.unwrap_or(contents);
        encrypt_blob(&cipher(), &mut std::io::Cursor::new(hashed), &mut blob).unwrap();
        FileJob {
            entry: Entry {
                path: "file".to_owned(),
                size: hashed.len() as u64,
                mtime: 0,
                hash: *blake3::hash(&blob).as_bytes(),
            },
            on_disk,
            root: Root {
                name: None,
                path: dir.to_path_buf(),
                is_dir: true,
                disk: None,
            },
            plaintext_hash: Some(*blake3::hash(hashed).as_bytes()),
        }
    }

    #[test]
    fn a_file_whose_root_is_gone_is_out_of_reach_not_changed() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = job(dir.path(), b"contents", None);
        let blob_file = dir.path().join("blob");
        // Deleted from a root that is there: changed (its record goes).
        std::fs::remove_file(&file.on_disk).unwrap();
        assert!(!write_blob(&cipher(), &file, &blob_file, &reading()).unwrap());
        // Its root gone (a linked disk unplugged): out of reach, kept.
        file.root = Root {
            name: Some("Films".to_owned()),
            path: dir.path().join("Films"),
            is_dir: true,
            disk: None,
        };
        file.on_disk = dir.path().join("Films/file");
        assert!(matches!(
            write_blob(&cipher(), &file, &blob_file, &reading()),
            Err(ShareError::SourceGone { name }) if name.as_deref() == Some("Films")
        ));
        assert!(!blob_file.exists());
    }

    #[test]
    fn a_blob_is_written_with_one_read_and_matches_the_catalogue() {
        let dir = tempfile::tempdir().unwrap();
        let file = job(dir.path(), b"the contents", None);
        let blob_file = dir.path().join("blob");
        assert!(write_blob(&cipher(), &file, &blob_file, &reading()).unwrap());
        let blob = std::fs::read(&blob_file).unwrap();
        assert_eq!(*blake3::hash(&blob).as_bytes(), file.entry.hash);
    }

    #[test]
    fn a_file_changed_since_the_catalogue_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        // Same size, other bytes: the nonce would belong to the old ones.
        let file = job(dir.path(), b"the CONTENTS", Some(b"the contents"));
        let blob_file = dir.path().join("blob");
        assert!(!write_blob(&cipher(), &file, &blob_file, &reading()).unwrap());
        assert!(!blob_file.exists());

        let mut gone = job(dir.path(), b"x", None);
        gone.on_disk = dir.path().join("missing");
        assert!(!write_blob(&cipher(), &gone, &blob_file, &reading()).unwrap());
        assert!(!blob_file.exists());
    }

    #[test]
    fn without_a_plaintext_hash_the_file_is_read_twice() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = job(dir.path(), b"the contents", None);
        file.plaintext_hash = None;
        let blob_file = dir.path().join("blob");
        assert!(write_blob(&cipher(), &file, &blob_file, &reading()).unwrap());
        let blob = std::fs::read(&blob_file).unwrap();
        assert_eq!(*blake3::hash(&blob).as_bytes(), file.entry.hash);
    }
}
