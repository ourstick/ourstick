//! The first half of every read: fetch and verify the latest signed record,
//! fetch and decrypt the manifest it names, check every path, and raise the
//! record floor (ADR 0004). `get` goes on to the files; `ls` stops here.

use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;

use super::files::check_manifest_paths;
use super::folder::plan_folder;
use super::{cancelled, DownloadError};
use crate::blob::decrypt_blob;
use crate::blocking::run_blocking;
use crate::cipher::{cipher_for, Cipher, CipherError, CipherId};
use crate::manifest::{check_manifest_blob_size, Manifest};
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

    // Its size is the owner's choice: stopped once it is too big to hold.
    let mut too_big = None;
    let fetched = connection
        .fetch_blob(
            &record.manifest_hash,
            &mut |received| match check_manifest_blob_size(received) {
                Ok(()) => GO,
                Err(error) => {
                    too_big = Some(error);
                    ControlFlow::Break(())
                }
            },
        )
        .await;
    if let Some(error) = too_big {
        return Err(error.into());
    }
    fetched.map_err(cancelled)?;
    // Also one that was here already, whole.
    check_manifest_blob_size(fetcher.local_bytes(&record.manifest_hash).await?)?;
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
    /// What it would write into the folder, files already there left out
    /// (a preview's; elsewhere its bytes).
    pub to_write: u64,
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
    /// Where it would be saved: as `choose` said, once it knew the name.
    pub folder: PathBuf,
    /// What would be saved under another name there (ADR 0010).
    pub saved_as: Vec<super::SavedAs>,
    /// The manifest, kept for what is inside a folder when the window opens
    /// it ([`entries_in`]), and what each path would write.
    pub manifest: Manifest,
    pub to_write_by_path: BTreeMap<String, u64>,
}

/// Fetches and verifies the stick's manifest and says what adding it would
/// bring into the folder `choose` gives for its name (once, with one
/// connection); nothing is written. Like [`list`], it raises the
/// stick's record floor. No deadline of its own: the caller bounds it (the
/// sender may be away), and a folder holding an earlier copy is read.
pub async fn preview(
    fetcher: &impl Fetcher,
    store: Arc<dyn Store>,
    ticket: &Ticket,
    choose: impl FnOnce(&str) -> PathBuf + Send + 'static,
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
    // Shared with the plan below, not copied.
    let manifest = std::sync::Arc::new(manifest);
    let total = manifest.total_size();
    let mut entries = top_entries(&manifest);
    // What the folder holds counts, as a download would find it (each file
    // not recorded there is read once: a big existing copy takes a while).
    let (to_write, folder, saved_as, by_top) = {
        let store = store.clone();
        let stick_id = ticket.stick_id;
        let manifest = std::sync::Arc::clone(&manifest);
        type Planned = (
            u64,
            PathBuf,
            Vec<super::SavedAs>,
            Option<BTreeMap<String, u64>>,
        );
        run_blocking(move || -> Result<Planned, DownloadError> {
            // Chosen here, off the async runtime: it may look at folders.
            let folder = choose(&manifest.name);
            // The names it would change are the same as a download's.
            let planned = |recorded: &[crate::store::DownloadedFile]| {
                plan_folder(&folder, &manifest, recorded, &[], cipher.as_ref())
            };
            match std::fs::metadata(&folder) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    // A new folder: every entry writes all of its bytes.
                    let plan = planned(&[])?;
                    return Ok((manifest.total_size(), folder.clone(), plan.renamed, None));
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
            let plan = planned(&recorded)?;
            Ok((
                plan.bytes_to_write(&manifest),
                folder.clone(),
                plan.renamed.clone(),
                Some(plan.bytes_by_top(&manifest)),
            ))
        })
        .await?
    };
    if let Some(by_top) = &by_top {
        for entry in &mut entries {
            entry.to_write = by_top.get(&entry.name).copied().unwrap_or(0);
        }
    }
    Ok(Preview {
        version,
        name: manifest.name.clone(),
        partial: manifest.partial,
        entries,
        total,
        to_write,
        folder,
        saved_as,
        // No plan when the folder is new: everything is written.
        to_write_by_path: by_top.unwrap_or_else(|| all_bytes_by_path(&manifest)),
        // The plan's share has ended: taken back without a copy.
        manifest: std::sync::Arc::unwrap_or_clone(manifest),
    })
}

/// Every file's bytes by each top-level path and each path inside one (as
/// `FolderPlan::bytes_by_top`, with every file to write).
fn all_bytes_by_path(manifest: &Manifest) -> BTreeMap<String, u64> {
    let mut by_path: BTreeMap<String, u64> = BTreeMap::new();
    for file in &manifest.entries {
        let mut parts = file.path.splitn(3, '/');
        let first = parts.next().unwrap_or_default();
        let mut keys = vec![first.to_owned()];
        if let Some(second) = parts.next() {
            keys.push(format!("{first}/{second}"));
        }
        for key in keys {
            let bytes = by_path.entry(key).or_default();
            *bytes = bytes.saturating_add(file.size);
        }
    }
    by_path
}

/// The manifest's files grouped by their first component, in name order.
pub(super) fn top_entries(manifest: &Manifest) -> Vec<TopEntry> {
    entries_in(manifest, None, None)
}

/// What is in `folder` (a top-level folder of the stick), or at the top
/// level with `None`: its folders and files, in name order, each named by
/// its whole path (`Folder/Sub`), so a choice is made with it as it is
/// (ADR 0012, Amendment 1). `to_write`, a preview's bytes to write by path
/// (`FolderPlan::bytes_by_top`), fills what each would write; else its
/// bytes.
pub fn entries_in(
    manifest: &Manifest,
    folder: Option<&str>,
    to_write: Option<&BTreeMap<String, u64>>,
) -> Vec<TopEntry> {
    let inside = folder.map(|folder| format!("{folder}/"));
    let mut entries: BTreeMap<String, TopEntry> = BTreeMap::new();
    for file in &manifest.entries {
        let rest = match &inside {
            Some(inside) => match file.path.strip_prefix(inside.as_str()) {
                Some(rest) => rest,
                None => continue,
            },
            None => file.path.as_str(),
        };
        let (first, deeper) = match rest.split_once('/') {
            Some((first, _)) => (first, true),
            None => (rest, false),
        };
        let name = match folder {
            Some(folder) => format!("{folder}/{first}"),
            None => first.to_owned(),
        };
        let entry = entries.entry(name.clone()).or_insert_with(|| TopEntry {
            name,
            is_dir: deeper,
            files: 0,
            bytes: 0,
            to_write: 0,
        });
        entry.is_dir |= deeper;
        entry.files += 1;
        entry.bytes = entry.bytes.saturating_add(file.size);
    }
    let mut entries: Vec<TopEntry> = entries.into_values().collect();
    for entry in &mut entries {
        entry.to_write = match to_write {
            Some(by_path) => by_path.get(&entry.name).copied().unwrap_or(0),
            None => entry.bytes,
        };
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Entry;

    fn manifest(paths: &[(&str, u64)]) -> Manifest {
        Manifest {
            entries: paths
                .iter()
                .map(|(path, size)| Entry {
                    path: (*path).to_owned(),
                    size: *size,
                    mtime: 0,
                    hash: [0; 32],
                })
                .collect(),
            ..Manifest::default()
        }
    }

    #[test]
    fn what_is_in_one_folder_is_named_by_its_whole_path() {
        let stick = manifest(&[
            ("Disk/MUSIC/a.flac", 4),
            ("Disk/MUSIC/deeper/b.flac", 2),
            ("Disk/readme.txt", 1),
            ("Diskette/x", 9),
            ("Films/a.mkv", 7),
        ]);
        let top = entries_in(&stick, None, None);
        let names: Vec<&str> = top.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["Disk", "Diskette", "Films"]);
        let inside = entries_in(&stick, Some("Disk"), None);
        let named: Vec<(&str, bool, u64, u64)> = inside
            .iter()
            .map(|entry| (entry.name.as_str(), entry.is_dir, entry.files, entry.bytes))
            .collect();
        assert_eq!(
            named,
            [("Disk/MUSIC", true, 2, 6), ("Disk/readme.txt", false, 1, 1)]
        );
        // What a preview says each would write.
        let to_write: BTreeMap<String, u64> = [("Disk/MUSIC".to_owned(), 2)].into();
        let inside = entries_in(&stick, Some("Disk"), Some(&to_write));
        assert_eq!(inside[0].to_write, 2);
        assert_eq!(inside[1].to_write, 0);
        assert!(entries_in(&stick, Some("Nothing"), None).is_empty());
        // A new folder: no plan, every file written.
        let all = all_bytes_by_path(&stick);
        assert_eq!(all.get("Disk"), Some(&7));
        assert_eq!(all.get("Disk/MUSIC"), Some(&6));
        assert_eq!(all.get("Films/a.mkv"), Some(&7));
    }
}
