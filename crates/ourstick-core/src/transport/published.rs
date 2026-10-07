//! The version an owner published, handed to its files keeper (ADR 0019).

use std::sync::Arc;

use tokio::sync::watch;

use super::BlobHash;
use crate::manifest::Manifest;

/// A version as published: its manifest's hash and the manifest, decoded
/// once and shared (ADR 0019).
#[derive(Debug, Clone)]
pub struct PublishedVersion {
    pub manifest_hash: BlobHash,
    pub manifest: Arc<Manifest>,
}

/// For publishers: tells the owner's files keeper, through `published`,
/// of the version `manifest_hash`, unless it is the one it has. Returns
/// the manifest kept: the one it has, if the same version.
pub fn hand_over(
    published: &watch::Sender<Option<PublishedVersion>>,
    manifest_hash: BlobHash,
    manifest: Arc<Manifest>,
) -> Arc<Manifest> {
    let mut kept = manifest.clone();
    published.send_if_modified(|version| {
        if let Some(version) = version
            .as_ref()
            .filter(|version| version.manifest_hash == manifest_hash)
        {
            kept = version.manifest.clone();
            return false;
        }
        *version = Some(PublishedVersion {
            manifest_hash,
            manifest,
        });
        true
    });
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Arc<Manifest> {
        Arc::new(Manifest::empty())
    }

    #[tokio::test]
    async fn the_keeper_is_told_of_each_new_version_once() {
        let published = watch::Sender::new(None);
        let mut versions = published.subscribe();
        let first = hand_over(&published, [1; 32], manifest());
        assert!(versions.has_changed().unwrap());
        let version = versions.borrow_and_update().clone().unwrap();
        assert_eq!(version.manifest_hash, [1; 32]);
        assert!(Arc::ptr_eq(&first, &version.manifest));
        // The same version again (an unchanged folder): nothing to tell,
        // and the manifest kept is the first, not a second copy.
        let again = hand_over(&published, [1; 32], manifest());
        assert!(!versions.has_changed().unwrap());
        assert!(Arc::ptr_eq(&again, &first));
        hand_over(&published, [2; 32], manifest());
        assert!(versions.has_changed().unwrap());
        assert_eq!(versions.borrow().as_ref().unwrap().manifest_hash, [2; 32]);
    }

    #[test]
    fn a_version_is_kept_with_no_one_told() {
        let published = watch::Sender::new(None);
        hand_over(&published, [3; 32], manifest());
        let versions = published.subscribe();
        assert_eq!(versions.borrow().as_ref().unwrap().manifest_hash, [3; 32]);
    }
}
