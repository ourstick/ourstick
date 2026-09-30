//! The reader's blob stores, one per batch (ADR 0007): each batch is an
//! iroh-blobs `FsStore` in its own folder under the stick's store folder, and
//! leaves by having its folder removed. iroh-blobs 0.103 has no public way to
//! delete one blob, and its GC would also delete partial blobs, which hold the
//! resume state.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use iroh_blobs::store::fs::FsStore;
use tokio::sync::Mutex;

use crate::transport::{is_batch_name, TransportError, MANIFEST_BATCH};

fn storage(error: impl std::fmt::Display) -> TransportError {
    TransportError::Storage(error.to_string())
}

fn checked(name: &str) -> Result<&str, TransportError> {
    if is_batch_name(name) {
        Ok(name)
    } else {
        Err(TransportError::Storage(format!(
            "not a batch name: {name:?}"
        )))
    }
}

/// The batch in use, shared by the fetcher and its connections.
#[derive(Clone)]
pub(super) struct Batches {
    root: PathBuf,
    current: Arc<Mutex<(String, FsStore)>>,
}

async fn open(root: &Path, name: &str) -> Result<FsStore, TransportError> {
    let dir = root.join(name);
    tokio::fs::create_dir_all(&dir).await?;
    FsStore::load(&dir).await.map_err(storage)
}

impl Batches {
    /// Opens the stick's store folder `root`, with [`MANIFEST_BATCH`] in use.
    pub(super) async fn open(root: PathBuf) -> Result<Self, TransportError> {
        tokio::fs::create_dir_all(&root).await?;
        let store = open(&root, MANIFEST_BATCH).await?;
        Ok(Self {
            root,
            current: Arc::new(Mutex::new((MANIFEST_BATCH.to_owned(), store))),
        })
    }

    /// The store of the batch in use.
    pub(super) async fn store(&self) -> FsStore {
        self.current.lock().await.1.clone()
    }

    pub(super) async fn use_batch(&self, name: &str) -> Result<(), TransportError> {
        let name = checked(name)?;
        let mut current = self.current.lock().await;
        if current.0 != name {
            current.1.shutdown().await.map_err(storage)?;
            *current = (name.to_owned(), open(&self.root, name).await?);
        }
        Ok(())
    }

    pub(super) async fn drop_batch(&self, name: &str) -> Result<(), TransportError> {
        let name = checked(name)?;
        let mut current = self.current.lock().await;
        if current.0 == name {
            current.1.shutdown().await.map_err(storage)?;
            *current = (
                MANIFEST_BATCH.to_owned(),
                open(&self.root, MANIFEST_BATCH).await?,
            );
        }
        remove_dir(&self.root.join(name)).await
    }

    pub(super) async fn keep_only(&self, keep: &[String]) -> Result<(), TransportError> {
        let current = self.current.lock().await;
        let mut entries = tokio::fs::read_dir(&self.root).await?;
        while let Some(entry) = entries.next_entry().await? {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let kept = name == MANIFEST_BATCH || name == current.0 || keep.contains(&name);
            if !kept && is_batch_name(&name) && entry.file_type().await?.is_dir() {
                remove_dir(&entry.path()).await?;
            }
        }
        Ok(())
    }

    /// Closes the store in use; with `keep = false`, deletes every batch.
    pub(super) async fn shutdown(self, keep: bool) -> Result<(), TransportError> {
        self.current
            .lock()
            .await
            .1
            .shutdown()
            .await
            .map_err(storage)?;
        if !keep {
            remove_dir(&self.root).await?;
        }
        Ok(())
    }
}

async fn remove_dir(dir: &Path) -> Result<(), TransportError> {
    match tokio::fs::remove_dir_all(dir).await {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}
