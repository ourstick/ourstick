//! The latest manifest, kept as a file in the stick's cache folder: for
//! serving the latest version again at once after a start, before a pass
//! reads the folder (otherwise readers would find nothing to fetch until the
//! pass publishes, minutes later for a big folder), and for the owner's
//! files keeper, which needs to know the current files (ADR 0017).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{Owner, Publication, ShareError};
use crate::blob::decrypt_blob;
use crate::blocking::run_blocking;
use crate::cipher::cipher_for;
use crate::manifest::Manifest;
use crate::transport::Publisher;

/// Where the latest manifest blob is kept.
fn latest_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("latest-manifest.blob")
}

/// Keeps `blob` as the latest manifest, replacing the one before in one
/// step. Blocking.
pub(super) fn save_latest_manifest(cache_dir: &Path, blob: &[u8]) -> Result<(), ShareError> {
    let path = latest_path(cache_dir);
    let temporary = cache_dir.join("latest-manifest.blob.new");
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| ShareError::Io { path, source }
    };
    std::fs::write(&temporary, blob).map_err(io(&temporary))?;
    std::fs::rename(&temporary, &path).map_err(io(&path))
}

/// The latest manifest blob of the stick whose cache folder is `cache_dir`,
/// if one is kept and its hash is `hash`. Blocking.
pub fn load_latest_manifest(cache_dir: &Path, hash: &[u8; 32]) -> Option<Vec<u8>> {
    let blob = std::fs::read(latest_path(cache_dir)).ok()?;
    (blake3::hash(&blob).as_bytes() == hash).then_some(blob)
}

/// Serves the latest signed version of the stick `owner` shares again, with
/// the manifest kept for it. `None` if the stick was never published or that
/// manifest is not kept: then only a pass can publish.
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
    let blob = {
        let cache_dir = owner.publisher.cache_dir().to_path_buf();
        run_blocking(move || load_latest_manifest(&cache_dir, &manifest_hash)).await
    };
    let Some(blob) = blob else {
        return Ok(None);
    };
    let cipher = cipher_for(owned.cipher, owner.stick_key);
    let (manifest_blob, manifest) =
        run_blocking(move || -> Result<(Vec<u8>, Arc<Manifest>), ShareError> {
            let mut plaintext = Vec::new();
            decrypt_blob(cipher.as_ref(), &mut blob.as_slice(), &mut plaintext)?;
            Ok((blob, Arc::new(Manifest::from_bytes(&plaintext)?)))
        })
        .await?;
    // Its files are "not ready" until the files keeper serves them.
    let manifest = owner
        .publisher
        .publish(record.clone(), manifest_blob, manifest)
        .await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_manifest_is_kept_and_checked() {
        let dir = tempfile::tempdir().unwrap();
        let hash = *blake3::hash(b"one").as_bytes();
        assert_eq!(load_latest_manifest(dir.path(), &hash), None);
        save_latest_manifest(dir.path(), b"one").unwrap();
        assert_eq!(
            load_latest_manifest(dir.path(), &hash),
            Some(b"one".to_vec())
        );
        save_latest_manifest(dir.path(), b"two").unwrap();
        // Another version's hash: not that one.
        assert_eq!(load_latest_manifest(dir.path(), &hash), None);
    }
}
