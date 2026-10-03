//! The row of the stick a download receives: written in one place, for the
//! program that receives it (ADR 0009's kinds; step 7b), and its name kept
//! from every verified manifest.

use std::path::Path;
use std::sync::Arc;

use super::DownloadError;
use crate::blocking::run_blocking;
use crate::stick_id::StickId;
use crate::store::{DownloadedFile, JoinedChange, JoinedStick, StickKind, Store, StoreError};
use crate::ticket::Ticket;

/// Records the stick as received by `kind` into `output_dir`: a new row,
/// or the folder of an earlier one (another folder holds nothing yet: it
/// replaces the old one). `get` never touches the app's stick; the app takes
/// over one `get` received. The caller holds the stick's download lock.
pub(super) async fn prepare(
    store: &Arc<dyn Store>,
    stick_id: StickId,
    output_dir: &Path,
    kind: StickKind,
    now_ms: u64,
) -> Result<(), DownloadError> {
    let store = store.clone();
    let output_dir = output_dir.to_path_buf();
    run_blocking(move || {
        match store.joined_stick(&stick_id)? {
            // The window's stick is the window's alone, also in its own
            // folder: paused there, it stays paused.
            Some(joined) if joined.kind == StickKind::App && kind == StickKind::Cli => {
                return Err(DownloadError::ReceivedByApp(joined.output_dir));
            }
            Some(joined) => {
                if joined.output_dir != output_dir {
                    store.move_joined_stick(&stick_id, &output_dir)?;
                }
                if joined.kind != kind {
                    store.change_joined_stick(&stick_id, &JoinedChange::Kind(kind))?;
                }
            }
            None => {
                let joined = JoinedStick {
                    kind,
                    ..JoinedStick::new(stick_id, output_dir, now_ms)
                };
                return match store.add_joined_stick(&joined) {
                    // Hidden: it is being forgotten.
                    Err(StoreError::StickExists) => Err(DownloadError::BeingForgotten),
                    added => Ok(added?),
                };
            }
        }
        Ok(())
    })
    .await
}

/// Keeps the stick's name as the verified manifest `name` gives it.
pub(super) async fn remember_name(
    store: &Arc<dyn Store>,
    joined: &JoinedStick,
    name: &str,
) -> Result<(), DownloadError> {
    if joined.name.as_deref() == Some(name) {
        return Ok(());
    }
    let store = store.clone();
    let stick_id = joined.stick_id;
    let name = name.to_owned();
    run_blocking(move || store.change_joined_stick(&stick_id, &JoinedChange::Name(name)))
        .await
        .map_err(DownloadError::from)
}

/// What this round's folder holds, file by file, and where each file goes,
/// with what is recorded of it and what the user chose not to get (read
/// every round: a choice saved while it waited counts now). Nothing is
/// fetched for a file that is there, and nothing written over a file
/// changed there.
pub(super) async fn plan_round(
    store: &Arc<dyn Store>,
    ticket: &Ticket,
    output_dir: &Path,
    manifest: &crate::manifest::Manifest,
    cipher: &Arc<dyn crate::cipher::Cipher>,
) -> Result<super::folder::FolderPlan, DownloadError> {
    let store = store.clone();
    let stick_id = ticket.stick_id;
    let (recorded, left_out) = run_blocking(move || {
        Ok::<_, StoreError>((
            store.downloaded_files(&stick_id)?,
            store.left_out(&stick_id)?,
        ))
    })
    .await?;
    let output_dir = output_dir.to_path_buf();
    let manifest = manifest.clone();
    let cipher = cipher.clone();
    run_blocking(move || {
        super::folder::plan_folder(
            &output_dir,
            &manifest,
            &recorded,
            &left_out,
            cipher.as_ref(),
        )
    })
    .await
}

/// What a round does once its folder is planned, before any file comes:
/// rows of files gone, or saved at a new place, are forgotten (their places
/// are free for this round, ADR 0010); then `Planned`, a `SavedAs` per name
/// saved under another, and the files found in place recorded.
pub(super) async fn take_plan(
    store: &Arc<dyn Store>,
    ticket: &Ticket,
    plan: &super::folder::FolderPlan,
    manifest: &crate::manifest::Manifest,
    on_event: &mut (dyn FnMut(super::DownloadEvent<'_>) -> std::ops::ControlFlow<()> + Send),
) -> Result<(), DownloadError> {
    use super::DownloadEvent;
    for path in plan.forget.clone() {
        let store = store.clone();
        let stick_id = ticket.stick_id;
        run_blocking(move || store.forget_downloaded_file(&stick_id, &path)).await?;
    }
    let bytes = plan.bytes_to_write(manifest);
    if on_event(DownloadEvent::Planned { bytes }).is_break() {
        return Err(DownloadError::Cancelled);
    }
    for renamed in &plan.renamed {
        let event = DownloadEvent::SavedAs {
            path: &renamed.path,
            saved_as: &renamed.saved_as,
            files: renamed.files,
        };
        if on_event(event).is_break() {
            return Err(DownloadError::Cancelled);
        }
    }
    for (name, files, bytes) in &plan.not_chosen {
        let event = DownloadEvent::NotChosen {
            name,
            files: *files,
            bytes: *bytes,
        };
        if on_event(event).is_break() {
            return Err(DownloadError::Cancelled);
        }
    }
    for group in plan.found.chunks(RECORD_GROUP) {
        record_downloaded(store, ticket, group.to_vec()).await?;
    }
    Ok(())
}

/// Files are recorded this many in one transaction (ADR 0014): one each,
/// the commits' syncs were most of the time of a stick of small files. The
/// owner's catalogue records the same number a transaction; a power cut
/// loses at most this many records, found again by content.
pub(super) const RECORD_GROUP: usize = 256;

/// Records that `files` are in the stick's folder as written, all in one
/// transaction (at most [`RECORD_GROUP`] of them, by the callers).
pub(super) async fn record_downloaded(
    store: &Arc<dyn Store>,
    ticket: &Ticket,
    files: Vec<DownloadedFile>,
) -> Result<(), DownloadError> {
    if files.is_empty() {
        return Ok(());
    }
    let store = store.clone();
    let stick_id = ticket.stick_id;
    run_blocking(move || store.record_downloaded_files(&stick_id, &files)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryStore;

    fn setup() -> (Arc<dyn Store>, StickId) {
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
        let stick_id = crate::stick_id::StickSigningKey::from_bytes(&[1; 32]).stick_id();
        (store, stick_id)
    }

    #[tokio::test]
    async fn get_never_touches_the_windows_stick() {
        let (store, stick_id) = setup();
        let here = Path::new("/d/Holiday");
        prepare(&store, stick_id, here, StickKind::App, 1)
            .await
            .unwrap();
        for folder in [here, Path::new("/d/other")] {
            match prepare(&store, stick_id, folder, StickKind::Cli, 2).await {
                Err(DownloadError::ReceivedByApp(named)) => assert_eq!(named, here),
                other => panic!("{other:?}"),
            }
        }
        let joined = store.joined_stick(&stick_id).unwrap().unwrap();
        assert_eq!(
            (joined.kind, joined.output_dir.as_path()),
            (StickKind::App, here)
        );
    }

    #[tokio::test]
    async fn the_window_takes_over_what_get_received() {
        let (store, stick_id) = setup();
        prepare(&store, stick_id, Path::new("/d/a"), StickKind::Cli, 1)
            .await
            .unwrap();
        prepare(&store, stick_id, Path::new("/d/b"), StickKind::App, 2)
            .await
            .unwrap();
        let joined = store.joined_stick(&stick_id).unwrap().unwrap();
        assert_eq!(joined.kind, StickKind::App);
        assert_eq!(joined.output_dir, Path::new("/d/b"));
    }

    #[tokio::test]
    async fn a_stick_being_forgotten_is_not_received_again_meanwhile() {
        let (store, stick_id) = setup();
        prepare(&store, stick_id, Path::new("/d/a"), StickKind::App, 1)
            .await
            .unwrap();
        store
            .change_joined_stick(&stick_id, &JoinedChange::Forgetting)
            .unwrap();
        assert!(matches!(
            prepare(&store, stick_id, Path::new("/d/a"), StickKind::App, 2).await,
            Err(DownloadError::BeingForgotten)
        ));
    }

    #[tokio::test]
    async fn the_name_follows_the_manifest() {
        let (store, stick_id) = setup();
        prepare(&store, stick_id, Path::new("/d/a"), StickKind::App, 1)
            .await
            .unwrap();
        let joined = store.joined_stick(&stick_id).unwrap().unwrap();
        remember_name(&store, &joined, "Holiday").await.unwrap();
        let joined = store.joined_stick(&stick_id).unwrap().unwrap();
        assert_eq!(joined.name.as_deref(), Some("Holiday"));
        // The same name again writes nothing (and fails nothing).
        remember_name(&store, &joined, "Holiday").await.unwrap();
    }
}
