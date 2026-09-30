//! Serving the latest version again at once after a start, before a pass
//! reads the folder: otherwise readers would find nothing to fetch until the
//! pass publishes, minutes later for a big folder.

use std::collections::HashSet;

use super::{Owner, Publication, ShareError};
use crate::blob::decrypt_blob;
use crate::blocking::run_blocking;
use crate::cipher::cipher_for;
use crate::manifest::Manifest;
use crate::transport::{BlobHash, Publisher};

/// Serves the latest signed version of the stick `owner` shares again, with
/// the manifest the cache still holds for it. `None` if the stick was never
/// published or the cache no longer holds that manifest: then only a pass
/// can publish.
pub async fn serve_latest(
    owner: &Owner<'_, impl Publisher>,
) -> Result<Option<Publication>, ShareError> {
    let stick_id = owner.signing_key.stick_id();
    let owned = {
        let store = owner.store.clone();
        run_blocking(move || store.owned_stick(&stick_id)).await?
    }
    .ok_or(ShareError::NotOwned)?;
    let Some(record) = owned.latest_record else {
        return Ok(None);
    };
    let content = record.unverified_record();
    let manifest_hash = content.manifest_hash;
    let seq = content.seq;
    // Kept first, then read: the cache's GC cannot slip in between.
    if !owner.publisher.keep_blob(&manifest_hash).await? {
        return Ok(None);
    }
    let Some(blob) = owner.publisher.read_blob(&manifest_hash).await? else {
        return Ok(None);
    };
    let cipher = cipher_for(owned.cipher, owner.stick_key);
    let manifest = run_blocking(move || -> Result<Manifest, ShareError> {
        let mut plaintext = Vec::new();
        decrypt_blob(cipher.as_ref(), &mut blob.as_slice(), &mut plaintext)?;
        Ok(Manifest::from_bytes(&plaintext)?)
    })
    .await?;
    let allowed: HashSet<BlobHash> = manifest.entries.iter().map(|entry| entry.hash).collect();
    // The blobs the cache still holds are served at once; the others are
    // "not ready" until the pass fills them.
    for hash in &allowed {
        owner.publisher.keep_blob(hash).await?;
    }
    owner.publisher.publish(record.clone(), allowed).await?;
    tracing::info!(
        seq,
        partial = manifest.partial,
        "serving the latest version again"
    );
    Ok(Some(Publication {
        manifest,
        manifest_hash,
        seq,
        record,
        new_version: false,
        reused: 0,
    }))
}
