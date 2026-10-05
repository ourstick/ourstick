//! The owner's store's commands (ADR 0017): `ExportBao` is served, a few
//! that ask nothing of it are answered, every other is refused with an error.

use std::io;

use iroh_blobs::api::blobs::Bitfield;
use iroh_blobs::api::proto::{BlobStatus, Command, ExportBaoMsg, ExportBaoRequest};
use iroh_blobs::api::TempTag;

use super::{export_bao, Served};

pub(super) async fn run(
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
            // Dropped unanswered: its caller gets "sender closed", an error
            // (batches import, which this store never does).
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
