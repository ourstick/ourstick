//! The first half of every read: fetch and verify the latest signed record,
//! fetch and decrypt the manifest it names, check every path, and raise the
//! record floor (ADR 0004). `get` goes on to the files; `ls` stops here.

use std::sync::Arc;

use super::files::check_manifest_paths;
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
