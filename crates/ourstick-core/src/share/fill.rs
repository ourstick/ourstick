//! Filling the cache behind the catalogue (ADR 0007, until suitcases): each
//! catalogued file's blob is made again by a full encryption and enters the
//! cache only if its hash is the catalogue's. A blob the cache still holds is
//! kept as it is: a blob stored under a hash is exactly the bytes of that
//! hash.

use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::ShareError;
use crate::blocking::run_blocking;
use crate::cipher::{Cipher, CipherError};
use crate::manifest::build::hash_file;
use crate::manifest::{CataloguedFile, ManifestError};
use crate::transport::{BlobHash, Publisher};

/// The blobs of one pass that are in the cache, and how they got there.
pub(super) struct Fill<'a, P> {
    pub(super) publisher: &'a P,
    pub(super) cipher: Arc<dyn Cipher>,
    pub(super) root: PathBuf,
    /// Blobs of this pass known to be in the cache (identical files share
    /// one).
    pub(super) ready: HashSet<BlobHash>,
}

/// Where a manifest path is under `root`.
fn on_disk(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

impl<P: Publisher> Fill<'_, P> {
    /// Makes sure the blob of `file` is in the cache. False if the file no
    /// longer gives that blob (changed or gone since it was catalogued): it
    /// is left out, and the next pass takes it again.
    pub(super) async fn ensure(&mut self, file: &CataloguedFile) -> Result<bool, ShareError> {
        let hash = file.entry.hash;
        if self.ready.contains(&hash) || self.publisher.keep_blob(&hash).await? {
            self.ready.insert(hash);
            return Ok(true);
        }
        let made = {
            let root = self.root.clone();
            let path = on_disk(&root, &file.entry.path);
            let cipher = self.cipher.clone();
            let staging = self.publisher.staging_dir().to_path_buf();
            run_blocking(
                move || match hash_file(&root, &path, cipher.as_ref(), Some(&staging)) {
                    Ok((entry, blob)) => Ok(Some((entry.hash, blob))),
                    Err(ManifestError::Io { source, .. })
                        if source.kind() == ErrorKind::NotFound =>
                    {
                        Ok(None)
                    }
                    Err(ManifestError::Blob {
                        source: CipherError::PlaintextChanged,
                        ..
                    }) => Ok(None),
                    Err(error) => Err(ShareError::from(error)),
                },
            )
            .await?
        };
        let Some((made_hash, Some(blob))) = made else {
            return Ok(false);
        };
        // Checked before the blob enters the cache: the invariant of ADR 0007.
        let result = if made_hash == hash {
            let imported = self.publisher.import_blob(&blob).await?;
            if imported != hash {
                return Err(ShareError::CacheMismatch(blob));
            }
            self.ready.insert(hash);
            true
        } else {
            tracing::debug!("a file changed between catalogue and fill; left for the next pass");
            false
        };
        tokio::fs::remove_file(&blob)
            .await
            .map_err(|source| ShareError::Io { path: blob, source })?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_paths_map_to_the_disk() {
        let root = Path::new("/stick");
        assert_eq!(
            on_disk(root, "a/b/c.txt"),
            root.join("a").join("b").join("c.txt")
        );
        assert_eq!(on_disk(root, "top"), root.join("top"));
    }
}
