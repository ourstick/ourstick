//! Publishing one version of a stick: its manifest into the cache, a signed
//! record with the next `seq` (or the latest one again, for the same
//! manifest), and the blobs readers may ask for.

use std::collections::HashSet;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use super::{Publication, ShareError};
use crate::blob::encrypt_blob;
use crate::blocking::run_blocking;
use crate::cipher::Cipher;
use crate::clock::Clock;
use crate::manifest::Manifest;
use crate::record::{next_seq, SignedManifestRecord};
use crate::stick_id::StickSigningKey;
use crate::store::Store;
use crate::transport::{BlobHash, Publisher};

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
    /// Publishes `manifest`. Its blobs need not be in the cache yet: until
    /// they are, readers are told "not ready".
    pub(super) async fn publish(
        &self,
        signed: &mut Signed,
        manifest: Manifest,
        reused: usize,
    ) -> Result<Publication, ShareError> {
        let staging = self.publisher.staging_dir().to_path_buf();
        let (manifest_file, manifest_hash) = {
            let cipher = self.cipher.clone();
            let bytes = manifest.to_bytes()?;
            run_blocking(move || write_manifest_blob(cipher.as_ref(), &bytes, &staging)).await?
        };
        let imported = self.publisher.import_blob(&manifest_file).await?;
        if imported != manifest_hash {
            return Err(ShareError::CacheMismatch(manifest_file));
        }
        tokio::fs::remove_file(&manifest_file)
            .await
            .map_err(|source| ShareError::Io {
                path: manifest_file.clone(),
                source,
            })?;

        // The same manifest (same bytes, so also the same partial mark): the
        // same record, no new seq.
        let unchanged = signed
            .latest_record
            .clone()
            .filter(|record| record.unverified_record().manifest_hash == manifest_hash);
        let new_version = unchanged.is_none();
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
        let allowed: HashSet<BlobHash> = manifest.entries.iter().map(|entry| entry.hash).collect();
        self.publisher.publish(record.clone(), allowed).await?;
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

/// Encrypts the manifest into `staging/<hex hash>.blob`. Blocking.
fn write_manifest_blob(
    cipher: &dyn Cipher,
    bytes: &[u8],
    staging: &Path,
) -> Result<(std::path::PathBuf, BlobHash), ShareError> {
    let mut blob = Vec::new();
    encrypt_blob(cipher, &mut Cursor::new(bytes), &mut blob)?;
    let hash = *blake3::hash(&blob).as_bytes();
    let file = staging.join(format!("{}.blob", data_encoding::HEXLOWER.encode(&hash)));
    std::fs::write(&file, &blob).map_err(|source| ShareError::Io {
        path: file.clone(),
        source,
    })?;
    Ok((file, hash))
}
