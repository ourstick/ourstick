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
use iroh_blobs::api::blobs::Bitfield;
use iroh_blobs::api::proto::{BlobStatus, Command, ExportBaoMsg, ExportBaoRequest};
use iroh_blobs::api::{Store, TempTag};
use iroh_blobs::store::mem::MemStore;
use iroh_blobs::Hash;

use crate::blob::made::MadeBlob;
use crate::cipher::Cipher;
use crate::manifest::hashing::{outboard_path, BLOCK_SIZE};
use crate::transport::ServedFiles;

/// What the store serves, shared with the publisher, which swaps it with
/// each version.
#[derive(Clone)]
pub(super) struct Served {
    cipher: Arc<dyn Cipher>,
    outboards: PathBuf,
    /// None until the first version.
    files: Arc<Mutex<Option<Arc<ServedFiles>>>>,
}

impl Served {
    /// From now on, these files.
    pub(super) fn set(&self, files: ServedFiles) {
        *self.files.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(files));
    }

    fn now(&self) -> Option<Arc<ServedFiles>> {
        self.files.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn blob_len(&self, hash: &Hash) -> Option<u64> {
        self.now()?
            .files
            .get(hash.as_bytes())
            .map(|file| crate::blob::blob_len(self.cipher.as_ref(), file.plaintext_len))
    }
}

/// Starts the store of a stick whose outboards are in `outboards`. Its
/// exports run on `runtime` (threads of their own, ADR 0017 rule 8: they
/// read files and encrypt, which blocks).
pub(super) fn start(
    cipher: Arc<dyn Cipher>,
    outboards: PathBuf,
    runtime: tokio::runtime::Handle,
) -> (Store, Served) {
    let served = Served {
        cipher,
        outboards,
        files: Arc::default(),
    };
    let (sender, commands) = tokio::sync::mpsc::channel::<Command>(64);
    let store: Store = MemStore::from_sender(sender.into()).into();
    tokio::spawn(run(commands, served.clone(), runtime));
    (store, served)
}

async fn run(
    mut commands: tokio::sync::mpsc::Receiver<Command>,
    served: Served,
    runtime: tokio::runtime::Handle,
) {
    while let Some(command) = commands.recv().await {
        match command {
            Command::ExportBao(ExportBaoMsg {
                inner: ExportBaoRequest { hash, ranges },
                tx,
                ..
            }) => {
                let served = served.clone();
                runtime.spawn(async move { export_bao(&served, hash, ranges, tx).await });
            }
            Command::BlobStatus(cmd) => {
                let status = match served.blob_len(&cmd.inner.hash) {
                    Some(size) => BlobStatus::Complete { size },
                    None => BlobStatus::NotFound,
                };
                cmd.tx.send(status).await.ok();
            }
            Command::Observe(cmd) => {
                let bits = match served.blob_len(&cmd.inner.hash) {
                    Some(size) => Bitfield::complete(size),
                    None => Bitfield::empty(),
                };
                cmd.tx.send(bits).await.ok();
            }
            Command::Shutdown(cmd) => {
                cmd.tx.send(()).await.ok();
                break;
            }
            Command::WaitIdle(cmd) => {
                cmd.tx.send(()).await.ok();
            }
            Command::SyncDb(cmd) => {
                cmd.tx.send(Ok(())).await.ok();
            }
            Command::ClearProtected(cmd) => {
                cmd.tx.send(Ok(())).await.ok();
            }
            Command::CreateTempTag(cmd) => {
                cmd.tx.send(TempTag::new(cmd.inner.value, None)).await.ok();
            }
            Command::ListTags(cmd) => {
                cmd.tx.send(Vec::new()).await.ok();
            }
            Command::ListTempTags(cmd) => {
                cmd.tx.send(Vec::new()).await.ok();
            }
            // Everything that writes, deletes or lists: refused, never an
            // empty success (ADR 0017, rule 6).
            Command::ImportBao(cmd) => {
                cmd.tx.send(Err(unsupported().into())).await.ok();
            }
            Command::ImportBytes(cmd) => {
                cmd.tx.send(unsupported().into()).await.ok();
            }
            Command::ImportByteStream(cmd) => {
                cmd.tx.send(unsupported().into()).await.ok();
            }
            Command::ImportPath(cmd) => {
                cmd.tx.send(unsupported().into()).await.ok();
            }
            Command::ExportPath(cmd) => {
                cmd.tx
                    .send(iroh_blobs::api::Error::from(unsupported()).into())
                    .await
                    .ok();
            }
            Command::ExportRanges(cmd) => {
                cmd.tx
                    .send(iroh_blobs::api::proto::ExportRangesItem::Error(
                        unsupported().into(),
                    ))
                    .await
                    .ok();
            }
            Command::ListBlobs(cmd) => {
                cmd.tx.send(Err(unsupported().into())).await.ok();
            }
            Command::Batch(_) => {}
            Command::CreateTag(cmd) => {
                cmd.tx.send(Err(unsupported().into())).await.ok();
            }
            Command::RenameTag(cmd) => {
                cmd.tx.send(Err(unsupported().into())).await.ok();
            }
            Command::DeleteTags(cmd) => {
                cmd.tx.send(Err(unsupported().into())).await.ok();
            }
            Command::DeleteBlobs(cmd) => {
                cmd.tx.send(Err(unsupported().into())).await.ok();
            }
            Command::SetTag(cmd) => {
                cmd.tx.send(Err(unsupported().into())).await.ok();
            }
        }
    }
}

fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "this store only serves blobs")
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

struct ToSender(irpc::channel::mpsc::Sender<EncodedItem>);

impl bao_tree::io::mixed::Sender for ToSender {
    type Error = irpc::channel::SendError;
    async fn send(&mut self, item: EncodedItem) -> Result<(), Self::Error> {
        self.0.send(item).await
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
    let mut sender = ToSender(tx);
    if let Err(error) = export(served, hash, ranges, &mut sender).await {
        sender.0.send(failed(error)).await.ok();
    }
}

/// Serves `hash`: an error before anything is sent if it is not served, its
/// file cannot be opened or its outboard is not there; otherwise bao-tree
/// sends what it checked, and ends with its own error at the first group
/// that differs from the tree (a changed file).
async fn export(
    served: &Served,
    hash: Hash,
    ranges: ChunkRanges,
    sender: &mut ToSender,
) -> io::Result<()> {
    let files = served
        .now()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "nothing served yet"))?;
    let file = files
        .files
        .get(hash.as_bytes())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not served"))?;
    let path = files.layout.on_disk(&file.path);
    let blob = MadeBlob::open(
        &path,
        served.cipher.clone(),
        file.plaintext_len,
        &file.plaintext_hash,
    )?;
    // Size and root from the record (rule 2).
    let tree = BaoTree::new(blob.len(), BLOCK_SIZE);
    let root = blake3::Hash::from_bytes(*hash.as_bytes());
    if tree.size() <= BLOCK_SIZE.bytes() as u64 {
        // One group: checked against the root alone.
        let outboard = PreOrderMemOutboard {
            root,
            tree,
            data: Vec::new(),
        };
        return send(Data(blob), outboard, &ranges, sender).await;
    }
    let outboard = open_outboard(&served.outboards, &hash, tree)?;
    let outboard = PreOrderOutboard {
        root,
        tree,
        data: outboard,
    };
    send(Data(blob), outboard, &ranges, sender).await
}

async fn send(
    data: Data,
    outboard: impl Outboard,
    ranges: &ChunkRanges,
    sender: &mut ToSender,
) -> io::Result<()> {
    traverse_ranges_validated(data, outboard, ranges, sender)
        .await
        .map_err(|_| io::Error::other("the reader went away"))
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

#[cfg(test)]
#[path = "made_store_tests.rs"]
mod tests;
