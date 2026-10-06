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
    /// Through the disks' turns, unless the store was started without.
    reads: Option<Reads>,
}

// The table and the transport's ready set change together: each change of
// one is made, with `then`, under the table's lock, so they never disagree.
impl Served {
    /// From now on, these files, made with `cipher` (the stick's, given
    /// with every version so the store never keeps another); then
    /// `then(files)`, under the table's lock.
    pub(super) fn set(
        &self,
        cipher: Arc<dyn Cipher>,
        files: ServedFiles,
        then: impl FnOnce(&ServedFiles),
    ) {
        *self.cipher.lock().unwrap_or_else(|p| p.into_inner()) = cipher;
        let mut table = self.files.lock().unwrap_or_else(|p| p.into_inner());
        then(&files);
        *table = Some(files);
    }

    /// These files too, in the current version; `then(hash)` for each,
    /// under the table's lock.
    pub(super) fn add(&self, more: Vec<(BlobHash, ServedFile)>, then: impl Fn(Hash)) {
        let mut table = self.files.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(files) = table.as_mut() {
            for (hash, file) in more {
                files.files.insert(hash, file);
                then(Hash::from_bytes(hash));
            }
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

/// Starts the store of a stick whose outboards are in `outboards`, its
/// files read through `reads` (the disks' turns). Its exports run on
/// `runtime`, threads of their own (ADR 0017 rule 8): an export reads its
/// file and encrypts in its task, which blocks that thread, so a disk that
/// is slow, asleep or giving way holds one of them; nothing else runs
/// there.
pub(super) fn start(
    cipher: Arc<dyn Cipher>,
    outboards: PathBuf,
    runtime: tokio::runtime::Handle,
    on_not_served: OnNotServed,
    reads: Option<Reads>,
) -> (Store, Served) {
    let served = Served {
        cipher: Arc::new(Mutex::new(cipher)),
        outboards,
        files: Arc::default(),
        on_not_served,
        reads,
    };
    let (sender, commands) = tokio::sync::mpsc::channel::<Command>(64);
    let store: Store = MemStore::from_sender(sender.into()).into();
    // The commands are answered on the caller's runtime (nothing there
    // blocks), so they are while exports wait for a disk's turn; exports
    // run on `runtime`.
    let answering = tokio::runtime::Handle::try_current().unwrap_or_else(|_| runtime.clone());
    answering.spawn(run(commands, served.clone(), runtime));
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
    // Not in the version: nothing to tell.
    let (file, path) = match served.file(&hash) {
        Ok(found) => found,
        Err(error) => {
            sender.sender.send(failed(error)).await.ok();
            return;
        }
    };
    match export(served, hash, &file, &path, ranges, &mut sender).await {
        Err(Refused::ReaderGone) => {}
        // Not even begun: the file cannot be opened or its outboard is not
        // there. Not served from now on.
        Err(Refused::Unreadable(error)) => {
            tracing::debug!(%error, "a blob was not served");
            not_served(served, hash, &file.path, false);
            sender.sender.send(failed(error)).await.ok();
        }
        Ok(()) => {
            if let Some(changed) = sender.ended {
                tracing::debug!(changed, "a transfer ended early");
                not_served(served, hash, &file.path, changed);
            }
        }
    }
}

/// The store's [`Probe`](super::serving::Probe), run on `runtime`: whether
/// the file of `hash` is there, of its recorded size, with its outboard if
/// it needs one. If not, it is told as not served: changed if it is there
/// of another size, else could not be read (gone, or a disk unplugged).
pub(super) fn probe(served: Served, runtime: tokio::runtime::Handle) -> super::serving::Probe {
    let look = move |hash| {
        let Ok((file, path)) = served.file(&hash) else {
            return false;
        };
        let changed = match std::fs::metadata(&path) {
            Ok(metadata) if metadata.len() == file.plaintext_len => false,
            Ok(_) => true,
            Err(_) => {
                not_served(&served, hash, &file.path, false);
                return false;
            }
        };
        let tree = BaoTree::new(
            crate::blob::blob_len(served.cipher().as_ref(), file.plaintext_len),
            BLOCK_SIZE,
        );
        let can = !changed
            && (tree.outboard_size() == 0 || open_outboard(&served.outboards, &hash, tree).is_ok());
        if !can {
            not_served(&served, hash, &file.path, changed);
        }
        can
    };
    super::serving::Probe {
        look: Arc::new(look),
        runtime,
    }
}

/// `hash`, as served from `path`, is not served from now on, and the owner
/// is told; nothing if the table serves it from another path now (a newer
/// version, or a file made since).
fn not_served(served: &Served, hash: Hash, path: &str, changed: bool) {
    let mut table = served.files.lock().unwrap_or_else(|p| p.into_inner());
    let Some(files) = table.as_mut() else { return };
    if files
        .files
        .get(hash.as_bytes())
        .is_some_and(|file| &*file.path == path)
    {
        files.files.remove(hash.as_bytes());
        (served.on_not_served)(hash, changed);
    }
}

/// Serves `hash` from `file` at `path`: an error before anything is sent if
/// its file cannot be opened or its outboard is not there; otherwise
/// bao-tree sends what it checked, and ends with its own error at the first
/// group that differs from the tree (a changed file) or the first read that
/// fails.
async fn export(
    served: &Served,
    hash: Hash,
    file: &ServedFile,
    path: &std::path::Path,
    ranges: ChunkRanges,
    sender: &mut ToSender,
) -> Result<(), Refused> {
    let blob = MadeBlob::open(
        path,
        served.cipher(),
        file.plaintext_len,
        &file.plaintext_hash,
    )
    .and_then(|blob| match &served.reads {
        Some(reads) => reads.through_turns(blob, path),
        None => Ok(blob),
    })
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

/// Why an export did not begin, or ended early.
enum Refused {
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
mod reads;
use commands::run;
pub(super) use reads::Reads;

#[cfg(test)]
#[path = "made_store_tests.rs"]
mod tests;
