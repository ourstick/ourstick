//! Publishing one version of a stick: its manifest (kept as the latest), a signed
//! record with the next `seq` (or the latest one again, for the same
//! manifest), and the blobs readers may ask for.

use std::io::Cursor;
use std::sync::Arc;

use super::latest::{load_latest_manifest, save_latest_manifest};
use super::{Publication, ShareError};
use crate::blob::encrypt_blob;
use crate::blocking::run_blocking;
use crate::cipher::Cipher;
use crate::clock::Clock;
use crate::manifest::{check_manifest_blob_size, Manifest};
use crate::record::{next_seq, SignedManifestRecord};
use crate::stick_id::StickSigningKey;
use crate::store::Store;
use crate::transport::Publisher;

/// What was last signed for a stick, kept up to date through a pass.
#[derive(Debug, Clone)]
pub(super) struct Signed {
    pub(super) last_signed_seq: u64,
    pub(super) latest_record: Option<SignedManifestRecord>,
}

/// Where a version is published, and with what.
pub(super) struct Versions<'a, P> {
    pub(super) publisher: &'a P,
    pub(super) store: Arc<dyn Store>,
    pub(super) clock: &'a dyn Clock,
    pub(super) signing_key: &'a StickSigningKey,
    pub(super) cipher: Arc<dyn Cipher>,
}

impl<P: Publisher> Versions<'_, P> {
    /// Publishes `manifest`. Its blobs need not be ready: until the files
    /// keeper serves them, readers are told "not ready".
    pub(super) async fn publish(
        &self,
        signed: &mut Signed,
        manifest: Manifest,
        reused: usize,
    ) -> Result<Publication, ShareError> {
        // Off the async workers: a manifest of 146k files is tens of MB.
        let (manifest, manifest_blob, manifest_hash) = {
            let cipher = self.cipher.clone();
            run_blocking(move || -> Result<_, ShareError> {
                let bytes = manifest.to_bytes()?;
                let mut blob = Vec::new();
                encrypt_blob(cipher.as_ref(), &mut Cursor::new(bytes), &mut blob)?;
                // Never one that no reader would take.
                check_manifest_blob_size(blob.len() as u64)?;
                let hash = *blake3::hash(&blob).as_bytes();
                Ok((manifest, blob, hash))
            })
            .await?
        };

        // The same manifest (same bytes, so also the same partial mark): the
        // same record, no new seq.
        let unchanged = signed
            .latest_record
            .clone()
            .filter(|record| record.unverified_record().manifest_hash == manifest_hash);
        let new_version = unchanged.is_none();
        // For a restart (`serve_latest`): written again only if what is
        // kept is not this manifest already.
        {
            let cache_dir = self.publisher.cache_dir().to_path_buf();
            let blob = manifest_blob.clone();
            run_blocking(move || {
                if load_latest_manifest(&cache_dir, &manifest_hash).is_some() {
                    return Ok(());
                }
                save_latest_manifest(&cache_dir, &blob)
            })
            .await?;
        }
        let record = match unchanged {
            Some(record) => record,
            None => {
                let seq = next_seq(signed.last_signed_seq, self.clock.now_ms())
                    .ok_or(ShareError::SeqExhausted)?;
                let record = self.signing_key.sign_record(seq, manifest_hash);
                let store = self.store.clone();
                let saved = record.clone();
                let partial = manifest.partial;
                // Stored before it is served, so a seq is never used twice.
                run_blocking(move || store.save_signed_record(&saved, partial)).await?;
                signed.last_signed_seq = seq;
                signed.latest_record = Some(record.clone());
                record
            }
        };
        let manifest = self
            .publisher
            .publish(record.clone(), manifest_blob, Arc::new(manifest))
            .await?;
        let seq = record.unverified_record().seq;
        tracing::info!(
            seq,
            new_version,
            partial = manifest.partial,
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
}
