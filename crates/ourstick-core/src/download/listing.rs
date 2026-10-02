//! The first half of every read: fetch and verify the latest signed record,
//! fetch and decrypt the manifest it names, check every path, and raise the
//! record floor (ADR 0004). `get` goes on to the files; `ls` stops here.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use super::files::check_manifest_paths;
use super::folder::plan_folder;
use super::{cancelled, DownloadError};
use crate::blob::decrypt_blob;
use crate::blocking::run_blocking;
use crate::cipher::{cipher_for, Cipher, CipherError, CipherId};
use crate::manifest::Manifest;
use crate::record::{ManifestHint, RecordCheck};
use crate::store::{ManifestVersion, Store};
use crate::ticket::Ticket;
use crate::transport::{Fetcher, PeerConnection, GO, MANIFEST_BATCH};

/// A stick's current manifest, verified.
#[derive(Debug, Clone)]
pub struct Listing {
    /// The version of the record that named this manifest.
    pub version: ManifestVersion,
    pub manifest: Manifest,
}

/// Verifies the latest record against the ticket and `floor`, fetches and
/// decrypts its manifest, checks every path, and only then raises the floor.
/// Returns the manifest's cipher too, for the files.
pub(super) async fn fetch_listing(
    fetcher: &impl Fetcher,
    connection: &impl PeerConnection,
    store: &Arc<dyn Store>,
    ticket: &Ticket,
    floor: Option<ManifestVersion>,
) -> Result<(Listing, Arc<dyn Cipher>), DownloadError> {
    fetcher.use_batch(MANIFEST_BATCH).await?;
    let signed = connection.fetch_record(&ticket.stick_id).await?;
    let floor_hint = floor.map(|floor| ManifestHint {
        seq: floor.seq,
        hash: floor.hash,
    });
    let check = signed.check(&ticket.stick_id, &ticket.manifest_hint, floor_hint.as_ref())?;
    let (record, newer) = match check {
        RecordCheck::Newer(record) => (record, true),
        RecordCheck::Unchanged(record) => (record, false),
    };

    connection
        .fetch_blob(&record.manifest_hash, &mut |_| GO)
        .await
        .map_err(cancelled)?;
    let manifest_blob = fetcher.read_blob(&record.manifest_hash).await?;
    let cipher = cipher_of(&manifest_blob, ticket)?;
    let manifest = {
        let cipher = cipher.clone();
        run_blocking(move || -> Result<Manifest, DownloadError> {
            let mut plaintext = Vec::new();
            decrypt_blob(
                cipher.as_ref(),
                &mut manifest_blob.as_slice(),
                &mut plaintext,
            )?;
            Ok(Manifest::from_bytes(&plaintext)?)
        })
        .await?
    };
    // Before anything is accepted or written: every path must be safe here.
    check_manifest_paths(&manifest)?;
    if newer {
        // Accepted only once the manifest it names has been read successfully.
        let store = store.clone();
        run_blocking(move || store.accept_record(&record)).await?;
    }
    let version = ManifestVersion {
        seq: record.seq,
        hash: record.manifest_hash,
    };
    Ok((Listing { version, manifest }, cipher))
}

/// Lists the stick named in `ticket`: its current manifest, verified, with no
/// file data fetched and nothing written but the record floor (ADR 0004).
pub async fn list(
    fetcher: &impl Fetcher,
    store: Arc<dyn Store>,
    ticket: &Ticket,
) -> Result<Listing, DownloadError> {
    let floor = {
        let store = store.clone();
        let stick_id = ticket.stick_id;
        run_blocking(move || store.record_floor(&stick_id)).await?
    };
    let connection = fetcher.connect(ticket).await?;
    let result = fetch_listing(fetcher, &connection, &store, ticket, floor).await;
    connection.close().await;
    Ok(result?.0)
}

/// The stick's cipher, read from the manifest blob header. Every file blob must
/// use the same one (`decrypt_blob` checks).
fn cipher_of(manifest_blob: &[u8], ticket: &Ticket) -> Result<Arc<dyn Cipher>, DownloadError> {
    let byte = *manifest_blob.get(1).ok_or(CipherError::Truncated)?;
    Ok(cipher_for(CipherId::from_byte(byte)?, &ticket.stick_key))
}

/// One top-level file or folder of a stick, as a preview lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopEntry {
    pub name: String,
    pub is_dir: bool,
    pub files: u64,
    pub bytes: u64,
}

/// What adding a stick would bring (the receiver window, step 7b): its
/// name, its top-level entries with their sizes, and how much would be
/// written into `folder` (what is there already left out). A stick whose
/// owner is still reading its folder (`partial`) has more to come.
#[derive(Debug, Clone)]
pub struct Preview {
    pub version: ManifestVersion,
    pub name: String,
    pub partial: bool,
    pub entries: Vec<TopEntry>,
    pub total: u64,
    pub to_write: u64,
}

/// Fetches and verifies the stick's manifest and says what adding it into
/// `folder` would bring; nothing is written. Like [`list`], it raises the
/// stick's record floor. No deadline of its own: the caller bounds it (the
/// sender may be away), and a folder holding an earlier copy is read.
pub async fn preview(
    fetcher: &impl Fetcher,
    store: Arc<dyn Store>,
    ticket: &Ticket,
    folder: &Path,
) -> Result<Preview, DownloadError> {
    let floor = {
        let store = store.clone();
        let stick_id = ticket.stick_id;
        run_blocking(move || store.record_floor(&stick_id)).await?
    };
    let connection = fetcher.connect(ticket).await?;
    let fetched = fetch_listing(fetcher, &connection, &store, ticket, floor).await;
    connection.close().await;
    let (Listing { version, manifest }, cipher) = fetched?;
    let total = manifest.total_size();
    let entries = top_entries(&manifest);
    // What the folder holds counts, as a download would find it (each file
    // not recorded there is read once: a big existing copy takes a while).
    let to_write = {
        let store = store.clone();
        let stick_id = ticket.stick_id;
        let folder = folder.to_path_buf();
        let manifest = manifest.clone();
        run_blocking(move || -> Result<u64, DownloadError> {
            match std::fs::metadata(&folder) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(manifest.total_size())
                }
                Err(source) => {
                    return Err(DownloadError::Io {
                        path: folder,
                        source,
                    })
                }
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(DownloadError::Io {
                        path: folder,
                        source: std::io::Error::from(std::io::ErrorKind::NotADirectory),
                    })
                }
                Ok(_) => {}
            }
            let same = |joined: &crate::store::JoinedStick| {
                crate::places::comparable(&joined.output_dir) == crate::places::comparable(&folder)
            };
            let recorded = match store.joined_stick(&stick_id)? {
                Some(joined) if same(&joined) => store.downloaded_files(&stick_id)?,
                _ => Vec::new(),
            };
            let plan = plan_folder(&folder, &manifest, &recorded, cipher.as_ref())?;
            Ok(plan.bytes_to_write(&manifest))
        })
        .await?
    };
    Ok(Preview {
        version,
        name: manifest.name.clone(),
        partial: manifest.partial,
        entries,
        total,
        to_write,
    })
}

/// The manifest's files grouped by their first component, in name order.
fn top_entries(manifest: &Manifest) -> Vec<TopEntry> {
    let mut entries: BTreeMap<&str, TopEntry> = BTreeMap::new();
    for file in &manifest.entries {
        let (first, deeper) = match file.path.split_once('/') {
            Some((first, _)) => (first, true),
            None => (file.path.as_str(), false),
        };
        let entry = entries.entry(first).or_insert_with(|| TopEntry {
            name: first.to_owned(),
            is_dir: deeper,
            files: 0,
            bytes: 0,
        });
        entry.is_dir |= deeper;
        entry.files += 1;
        entry.bytes = entry.bytes.saturating_add(file.size);
    }
    entries.into_values().collect()
}
