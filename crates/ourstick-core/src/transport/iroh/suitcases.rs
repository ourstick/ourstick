//! Suitcases on iroh (ADR 0007): each is an iroh-blobs `FsStore` in its own
//! folder under `cache/suitcases/`, with no garbage collection. A blob enters
//! by reference to a file in that folder, and is served only if its hash is
//! the one expected. A suitcase leaves whole: once no transfer from it is in
//! progress, its store is closed and its folder removed. That is the only way
//! blobs leave; iroh-blobs 0.103 cannot delete one blob.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use iroh_blobs::api::blobs::{AddPathOptions, ImportMode};
use iroh_blobs::store::fs::options::{InlineOptions, Options};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, Hash};

use super::absolute;
use super::serving::Serving;
use crate::transport::{BlobHash, SuitcaseId, TransportError};

fn storage(error: impl std::fmt::Display) -> TransportError {
    TransportError::Storage(error.to_string())
}

/// The open suitcases of one publisher.
#[derive(Debug)]
pub(super) struct Suitcases {
    root: PathBuf,
    serving: Serving,
    /// The next id; ids below it were used.
    next: AtomicU64,
    open: Mutex<HashMap<SuitcaseId, (FsStore, PathBuf)>>,
}

impl Suitcases {
    /// Suitcases in `root`. Whatever an earlier run left there is removed
    /// first: suitcases are prepared again when asked for (ADR 0007). One
    /// program shares a stick at a time (its lock, `crate::locks`), and the
    /// folder is the stick's, so no one else's suitcases are here.
    pub(super) async fn start(root: PathBuf, serving: Serving) -> Result<Self, TransportError> {
        match tokio::fs::remove_dir_all(&root).await {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        tokio::fs::create_dir_all(&root).await?;
        Ok(Self {
            root,
            serving,
            next: AtomicU64::new(1),
            open: Mutex::new(HashMap::new()),
        })
    }

    fn open_suitcases(&self) -> std::sync::MutexGuard<'_, HashMap<SuitcaseId, (FsStore, PathBuf)>> {
        self.open
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// An id for a store that serves, never used before.
    pub(super) fn next_id(&self) -> SuitcaseId {
        self.next.fetch_add(1, Ordering::SeqCst)
    }

    pub(super) async fn open(&self) -> Result<(SuitcaseId, PathBuf), TransportError> {
        let id = self.next_id();
        let dir = self.root.join(id.to_string());
        let files = dir.join("files");
        tokio::fs::create_dir_all(&files).await?;
        let mut options = Options::new(&dir);
        options.gc = None;
        // Nothing inlined: a small blob would be copied into the store's
        // database, whose cache (1 GiB in redb) then grows with every small
        // file served. Referenced, it stays the file it is.
        options.inline = InlineOptions::NO_INLINE;
        let store = FsStore::load_with_opts(dir.join("blobs.db"), options)
            .await
            .map_err(storage)?;
        self.serving.add_store(id, (*store).clone());
        self.open_suitcases().insert(id, (store, dir));
        Ok((id, absolute(&files)?))
    }

    pub(super) async fn add(
        &self,
        id: SuitcaseId,
        blob_file: &Path,
        expected: &BlobHash,
    ) -> Result<bool, TransportError> {
        let (store, dir) = self
            .open_suitcases()
            .get(&id)
            .map(|(store, dir)| (store.clone(), dir.clone()))
            .ok_or_else(|| storage(format!("no suitcase {id}")))?;
        let path = absolute(blob_file)?;
        if !path.starts_with(&dir) {
            return Err(storage("a suitcase's blobs live in its own folder"));
        }
        let options = AddPathOptions {
            path,
            format: BlobFormat::Raw,
            // The file stays where it is until the suitcase goes: one write.
            mode: ImportMode::TryReference,
        };
        let tag = store
            .blobs()
            .add_path_with_opts(options)
            .temp_tag()
            .await
            .map_err(storage)?;
        let hash = tag.hash();
        // No GC in a suitcase: the blob stays without its tag.
        drop(tag);
        if *hash.as_bytes() != *expected {
            tracing::debug!("a blob did not have the expected hash; not served");
            return Ok(false);
        }
        self.serving.set_ready(id, Hash::from_bytes(*expected));
        Ok(true)
    }

    pub(super) async fn evict(&self, id: SuitcaseId) -> Result<(), TransportError> {
        // Waits until no transfer holds it; none can start any more.
        self.serving.remove_store(id).await;
        let Some((store, dir)) = self.open_suitcases().remove(&id) else {
            return Ok(());
        };
        store.shutdown().await.map_err(storage)?;
        drop(store);
        tokio::fs::remove_dir_all(&dir).await?;
        Ok(())
    }

    /// Closes every suitcase (at shutdown; their folders go at the next
    /// start).
    pub(super) async fn close_all(&self) {
        let open: Vec<FsStore> = self
            .open_suitcases()
            .drain()
            .map(|(_, (store, _))| store)
            .collect();
        for store in open {
            if let Err(error) = store.shutdown().await {
                tracing::warn!(%error, "a suitcase did not shut down cleanly");
            }
        }
    }
}
