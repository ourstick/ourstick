//! The owner's store (ADR 0017): one per stick, which iroh-blobs'
//! `handle_get` serves from. A blob's bytes are made from its file when they
//! are asked for ([`MadeBlob`]); bao-tree checks every 16 KiB group and
//! parent against the blob's tree before it is sent, with the size and the
//! root from the record and the outboard the catalogue kept, never from the
//! file (ADR 0017, rules 2 and 3). The store answers every other command of
//! iroh-blobs with an error: it only serves.

use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bao_tree::io::mixed::{traverse_ranges_validated, EncodedItem, ReadBytesAt};
use bao_tree::io::outboard::{PreOrderMemOutboard, PreOrderOutboard};
use bao_tree::io::sync::Outboard;
use bao_tree::{BaoTree, ChunkRanges};
use bytes::Bytes;
use iroh_blobs::api::proto::Command;
use iroh_blobs::api::Store;
use iroh_blobs::store::mem::MemStore;
use iroh_blobs::Hash;

use crate::blob::made::MadeBlob;
use crate::cipher::Cipher;
use crate::manifest::hashing::{outboard_path, BLOCK_SIZE};
use crate::transport::{BlobHash, ServedFile, ServedFiles};

/// Told when the store could not serve a blob: its hash, and whether its
/// file changed (or else could not be read).
pub(super) type OnNotServed = Arc<dyn Fn(Hash, bool) + Send + Sync>;

/// What the store serves, shared with the publisher: the version's table,
/// replaced with each version and added to as files are made ready.
#[derive(Clone)]
pub(super) struct Served {
    cipher: Arc<Mutex<Arc<dyn Cipher>>>,
    outboards: PathBuf,
    /// None until the first version.
    files: Arc<Mutex<Option<ServedFiles>>>,
    on_not_served: OnNotServed,
}

impl Served {
    /// From now on, these files, made with `cipher` (the stick's, given
    /// with every version so the store never keeps another).
    pub(super) fn set(&self, cipher: Arc<dyn Cipher>, files: ServedFiles) {
        *self.cipher.lock().unwrap_or_else(|p| p.into_inner()) = cipher;
        *self.files.lock().unwrap_or_else(|p| p.into_inner()) = Some(files);
    }

    /// These files too, in the current version.
    pub(super) fn add(&self, more: Vec<(BlobHash, ServedFile)>) {
        if let Some(files) = self
            .files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
        {
            files.files.extend(more);
        }
    }

    /// No longer `hash`.
    fn remove(&self, hash: &Hash) {
        if let Some(files) = self
            .files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
        {
            files.files.remove(hash.as_bytes());
        }
    }

    fn cipher(&self) -> Arc<dyn Cipher> {
        self.cipher
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// The file of `hash`, and where it is on disk: copied out, so the table
    /// is not held while the blob is sent.
    fn file(&self, hash: &Hash) -> io::Result<(ServedFile, PathBuf)> {
        let files = self.files.lock().unwrap_or_else(|p| p.into_inner());
        let files = files
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no version is served yet"))?;
        let file = files.files.get(hash.as_bytes()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "this blob is not in the version served",
            )
        })?;
        Ok((file.clone(), files.layout.on_disk(&file.path)))
    }

    fn blob_len(&self, hash: &Hash) -> Option<u64> {
        let plaintext_len = self
            .files
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()?
            .files
            .get(hash.as_bytes())?
            .plaintext_len;
        Some(crate::blob::blob_len(self.cipher().as_ref(), plaintext_len))
    }
}

/// Starts the store of a stick whose outboards are in `outboards`. It and
/// its exports run on `runtime`, threads of their own (ADR 0017 rule 8):
/// an export reads its file and encrypts in its task, which blocks that
/// thread, so a disk that is slow or asleep holds one of them; nothing
/// else runs there.
pub(super) fn start(
    cipher: Arc<dyn Cipher>,
    outboards: PathBuf,
    runtime: tokio::runtime::Handle,
    on_not_served: OnNotServed,
) -> (Store, Served) {
    let served = Served {
        cipher: Arc::new(Mutex::new(cipher)),
        outboards,
        files: Arc::default(),
        on_not_served,
    };
    let (sender, commands) = tokio::sync::mpsc::channel::<Command>(64);
    let store: Store = MemStore::from_sender(sender.into()).into();
    // On its own runtime, so it can be started from anywhere.
    runtime.spawn(run(commands, served.clone(), runtime.clone()));
    (store, served)
}

/// A made blob as bao-tree reads it.
struct Data(MadeBlob);

impl ReadBytesAt for Data {
    fn read_bytes_at(&self, offset: u64, size: usize) -> io::Result<Bytes> {
        let mut out = vec![0u8; size];
        self.0.read_at(offset, &mut out)?;
        Ok(out.into())
    }
}

/// Sends to the transfer, and notes whether bao-tree ended it because a
/// group or parent did not match the tree (the file changed) or because a
/// read failed.
struct ToSender {
    sender: irpc::channel::mpsc::Sender<EncodedItem>,
    ended: Option<bool>,
}

impl bao_tree::io::mixed::Sender for ToSender {
    type Error = irpc::channel::SendError;
    async fn send(&mut self, item: EncodedItem) -> Result<(), Self::Error> {
        if let EncodedItem::Error(error) = &item {
            self.ended = Some(matches!(
                error,
                bao_tree::io::EncodeError::LeafHashMismatch(_)
                    | bao_tree::io::EncodeError::ParentHashMismatch(_)
                    | bao_tree::io::EncodeError::SizeMismatch
            ));
        }
        self.sender.send(item).await
    }
}

fn failed(error: io::Error) -> EncodedItem {
    EncodedItem::Error(bao_tree::io::EncodeError::Io(error))
}

async fn export_bao(
    served: &Served,
    hash: Hash,
    ranges: ChunkRanges,
    tx: irpc::channel::mpsc::Sender<EncodedItem>,
) {
    let mut sender = ToSender {
        sender: tx,
        ended: None,
    };
    match export(served, hash, ranges, &mut sender).await {
        // Not in the version: nothing to tell.
        Err(Refused::NotInVersion(error)) => {
            sender.sender.send(failed(error)).await.ok();
        }
        // Not even begun: the file cannot be opened or its outboard is not
        // there. Not served from now on.
        Err(Refused::ReaderGone) => {}
        Err(Refused::Unreadable(error)) => {
            tracing::debug!(%error, "a blob was not served");
            not_served(served, hash, false);
            sender.sender.send(failed(error)).await.ok();
        }
        Ok(()) => {
            if let Some(changed) = sender.ended {
                tracing::debug!(changed, "a transfer ended early");
                not_served(served, hash, changed);
            }
        }
    }
}

/// The store's [`Probe`](super::serving::Probe): whether the file of `hash`
/// is there, of its recorded size, with its outboard if it needs one; if
/// not, it is told as not served (could not be read).
pub(super) fn probe(served: Served) -> super::serving::Probe {
    super::serving::Probe(Arc::new(move |hash| {
        let can = served.file(&hash).is_ok_and(|(file, path)| {
            std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() == file.plaintext_len) && {
                let tree = BaoTree::new(
                    crate::blob::blob_len(served.cipher().as_ref(), file.plaintext_len),
                    BLOCK_SIZE,
                );
                tree.outboard_size() == 0 || open_outboard(&served.outboards, &hash, tree).is_ok()
            }
        });
        if !can && served.file(&hash).is_ok() {
            not_served(&served, hash, false);
        }
        can
    }))
}

fn not_served(served: &Served, hash: Hash, changed: bool) {
    served.remove(&hash);
    (served.on_not_served)(hash, changed);
}

/// Serves `hash`: an error before anything is sent if it is not served, its
/// file cannot be opened or its outboard is not there; otherwise bao-tree
/// sends what it checked, and ends with its own error at the first group
/// that differs from the tree (a changed file) or the first read that fails.
async fn export(
    served: &Served,
    hash: Hash,
    ranges: ChunkRanges,
    sender: &mut ToSender,
) -> Result<(), Refused> {
    let (file, path) = served.file(&hash).map_err(Refused::NotInVersion)?;
    let blob = MadeBlob::open(
        &path,
        served.cipher(),
        file.plaintext_len,
        &file.plaintext_hash,
    )
    .map_err(|error| {
        Refused::Unreadable(io::Error::new(
            error.kind(),
            format!("its file cannot be opened: {error}"),
        ))
    })?;
    // Size and root from the record (rule 2).
    let tree = BaoTree::new(blob.len(), BLOCK_SIZE);
    let root = blake3::Hash::from_bytes(*hash.as_bytes());
    if tree.size() <= BLOCK_SIZE.bytes() as u64 {
        // One group: checked against the root alone; an empty outboard would
        // be read out of bounds by a tree with parents.
        debug_assert_eq!(tree.outboard_size(), 0);
        let outboard = PreOrderMemOutboard {
            root,
            tree,
            data: Vec::new(),
        };
        return send(Data(blob), outboard, &ranges, sender).await;
    }
    let outboard = open_outboard(&served.outboards, &hash, tree).map_err(Refused::Unreadable)?;
    let outboard = PreOrderOutboard {
        root,
        tree,
        data: outboard,
    };
    send(Data(blob), outboard, &ranges, sender).await
}

/// Why an export did not begin.
enum Refused {
    /// The blob is not in the version served (or none is).
    NotInVersion(io::Error),
    /// The reader went away while it was sent: nothing is wrong with the
    /// blob, and nothing is told.
    ReaderGone,
    /// Its file or outboard could not be read.
    Unreadable(io::Error),
}

async fn send(
    data: Data,
    outboard: impl Outboard,
    ranges: &ChunkRanges,
    sender: &mut ToSender,
) -> Result<(), Refused> {
    traverse_ranges_validated(data, outboard, ranges, sender)
        .await
        .map_err(|_| Refused::ReaderGone)
}

/// The outboard the catalogue kept for `hash`, if it is there and of the
/// length its tree gives (a short one would make bao-tree panic).
fn open_outboard(dir: &std::path::Path, hash: &Hash, tree: BaoTree) -> io::Result<File> {
    let file = File::open(outboard_path(dir, hash.as_bytes()))?;
    if file.metadata()?.len() != tree.outboard_size() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the outboard is not of its blob's length",
        ));
    }
    Ok(file)
}

mod commands;
use commands::run;

#[cfg(test)]
#[path = "made_store_tests.rs"]
mod tests;
