//! A pasted sharing link's preview (step 7b): what the stick holds, where
//! it would be saved and whether it fits. Keeps nothing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ourstick_core::device::random_key;
use ourstick_core::download::preview;
use ourstick_core::received::{check_link, ReceivedError};
use ourstick_core::space::{available, SystemDisks};
use ourstick_core::stick_names::received_folder_name;
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, Network};
use ourstick_core::transport::Fetcher;
use ourstick_core::StickKind;
use serde::Serialize;
use tauri::State;

use super::commands::ticket_of;
use crate::state::AppState;

/// How long a preview (or one try of a wait, ADR 0025) waits for the
/// sender.
pub(super) const PREVIEW_WAIT: Duration = Duration::from_secs(30);

/// A top-level entry of a previewed stick.
#[derive(Debug, Serialize)]
pub struct EntryView {
    /// Exactly as the stick names it, its whole path: what a choice is
    /// made with.
    name: String,
    /// Its last part, escaped for the window (the sender's text).
    shown: String,
    is_dir: bool,
    files: u64,
    bytes: u64,
    /// What it would write into the folder (ADR 0012): the window sums the
    /// ticked ones.
    to_write: u64,
}

/// A preview, or the sender away: then the window offers to wait for it
/// (ADR 0025).
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewOutcome {
    Seen {
        preview: PreviewView,
    },
    SenderAway,
    /// A link this computer knows already (ADR 0028 §4): the window says
    /// so, with `Show it`.
    Known {
        /// "own" (it sends it), "received" or "waiting" (ADR 0025).
        known: &'static str,
        /// Its id, for `Show it`.
        id: String,
        name: String,
    },
}

/// What [`check_link`] refused, if it is a link known here: one's own, one
/// received, one waited for. Other refusals stay errors.
fn known_of(
    checked: Result<(), ReceivedError>,
    stick_id: &ourstick_core::StickId,
) -> Result<Option<PreviewOutcome>, ReceivedError> {
    let known = |known, name| {
        Some(PreviewOutcome::Known {
            known,
            id: crate::state::stick_hex(stick_id),
            name,
        })
    };
    match checked {
        Ok(()) => Ok(None),
        Err(ReceivedError::OwnStick(name)) => Ok(known("own", name)),
        Err(ReceivedError::AlreadyReceived(name)) => Ok(known("received", name)),
        Err(ReceivedError::Waiting) => Ok(known("waiting", String::new())),
        Err(other) => Err(other),
    }
}

/// What adding a pasted sharing link would bring.
#[derive(Debug, Serialize)]
pub struct PreviewView {
    name: String,
    /// The sender is still counting its files: more is to come.
    partial: bool,
    /// Every top-level entry: each must be choosable (ADR 0012).
    entries: Vec<EntryView>,
    total: u64,
    to_write: u64,
    /// Where it would be saved.
    folder: String,
    /// Received there earlier with the command line: it goes on there.
    earlier: bool,
    /// Received earlier in another folder (it moves; the files there stay).
    earlier_elsewhere: Option<String>,
    /// What is left there for it.
    left: u64,
    /// What would be saved under another name there (ADR 0010).
    saved_as: super::words::SavedAsList,
}

/// Where received sticks go: `parent`, else the Downloads folder.
fn parent_folder(parent: Option<&str>) -> Result<PathBuf, String> {
    match parent {
        Some(parent) => Ok(PathBuf::from(parent)),
        None => downloads(),
    }
}

/// A desktop's Downloads folder.
#[cfg(desktop)]
fn downloads() -> Result<PathBuf, String> {
    let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let home = std::env::var_os(home).ok_or("OurStick cannot find your home folder.")?;
    Ok(PathBuf::from(home).join("Downloads"))
}

/// Android: `Download/OurStick` in the phone's shared storage, which the
/// Files app shows (ADR 0027 §3). A folder left there by an earlier install
/// is not the app's any more: [`folder_for`] takes another name.
#[cfg(target_os = "android")]
fn downloads() -> Result<PathBuf, String> {
    let storage = std::env::var_os("EXTERNAL_STORAGE")
        .filter(|storage| !storage.is_empty())
        .unwrap_or_else(|| "/sdcard".into());
    Ok(PathBuf::from(storage).join("Download").join("OurStick"))
}

/// iOS: the app's `Documents`, which the Files app shows under "On My
/// iPhone › OurStick" (ADR 0027 §3).
#[cfg(target_os = "ios")]
fn downloads() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("OurStick cannot find its folder.")?;
    Ok(PathBuf::from(home).join("Documents"))
}

/// The folder a received stick goes into: one named after it in `parent`,
/// not taken already. Blocking (it looks at folders).
fn folder_for(name: &str, parent: &Path) -> PathBuf {
    parent.join(received_folder_name(name, |candidate| {
        parent.join(candidate).exists()
    }))
}

/// Whether `error` says the sender cannot be reached (off, OurStick closed
/// there, no network), as a receive would wait for it.
fn sender_away(error: &ourstick_core::download::DownloadError) -> bool {
    use ourstick_core::download::{after_error, Next, Waiting};
    matches!(
        after_error(error, None, Duration::ZERO),
        Next::Wait(Waiting::Sender(_), _)
    )
}

/// Where a received stick goes once its name is known: the folder a
/// command-line receive of it is in (`earlier`, it goes on there) unless
/// the user chose a `parent`; else one named after it in `parent`, or in
/// Downloads. One choice for a preview and a wait (ADR 0025).
pub(super) fn folder_choice(
    earlier: Option<PathBuf>,
    parent: Option<&str>,
) -> Result<impl Fn(&str) -> PathBuf + Clone + Send + 'static, String> {
    let chosen = parent.is_some();
    let parent = parent_folder(parent)?;
    Ok(move |name: &str| match &earlier {
        Some(folder) if !chosen => folder.clone(),
        _ => folder_for(name, &parent),
    })
}

/// The nearest folder of `path` that exists, to measure its disk.
fn existing(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|ancestor| ancestor.is_dir())
        .unwrap_or(path)
        .to_path_buf()
}

/// "Add a sharing link…": what the stick holds, where it would go and
/// whether it fits. Waits for the sender at most half a minute. A stick
/// received earlier with the command line goes on in the folder it is in.
#[tauri::command]
pub async fn preview_link(
    text: String,
    parent: Option<String>,
    state: State<'_, AppState>,
    receivers: State<'_, super::Receivers>,
) -> Result<PreviewOutcome, String> {
    let ticket = ticket_of(&text)?;
    let store = state.store.clone();
    let stick_id = ticket.stick_id;
    let (known, earlier) = super::work(move || {
        let known = known_of(
            check_link(store.as_ref(), &stick_id, StickKind::App),
            &stick_id,
        )?;
        Ok((
            known,
            store
                .joined_stick(&stick_id)?
                .map(|joined| joined.output_dir),
        ))
    })
    .await?;
    if let Some(known) = known {
        return Ok(known);
    }
    // A throwaway blob store: a preview keeps nothing.
    let hex: String = random_key()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let blobs = state.data_dir.join("previews").join(hex);
    let config = IrohConfig::new(random_key(), Network::Public);
    crate::local_network::may_bind(&state).await;
    let fetcher = IrohFetcher::start(&config, &blobs).await.map_err(|error| {
        tracing::warn!(%error, "a preview's networking");
        super::device_words::here(
            "This computer's network could not be started. Check it, then try again.".to_owned(),
        )
    })?;
    let choose = folder_choice(earlier.clone(), parent.as_deref())?;
    let looked = tokio::time::timeout(
        PREVIEW_WAIT,
        preview(&fetcher, state.store.clone(), &ticket, choose),
    )
    .await;
    let _ = fetcher.shutdown(false).await;
    let _ = tauri::async_runtime::spawn_blocking(move || std::fs::remove_dir_all(&blobs)).await;
    let previewed = match looked {
        Err(_) => return Ok(PreviewOutcome::SenderAway),
        // Away as a receive sees it (the one place that decides): it can be
        // waited for too.
        Ok(Err(error)) if sender_away(&error) => return Ok(PreviewOutcome::SenderAway),
        Ok(Err(error)) => {
            return Err(crate::receiving::stopped_words_then(
                &error,
                crate::receiving::AfterStop::LookAgain,
            ))
        }
        Ok(Ok(previewed)) => previewed,
    };
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let folder = previewed.folder.clone();
    let left = super::work(move || {
        let measured = existing(&folder);
        Ok(available(
            &SystemDisks,
            store.as_ref(),
            &data_dir,
            &measured,
            None,
        )?)
    })
    .await?;
    // Kept for opening its folders (ADR 0012, Amendment 1).
    receivers.previewed_now(super::holder::Previewed {
        manifest: std::sync::Arc::new(previewed.manifest),
        to_write: std::sync::Arc::new(previewed.to_write_by_path),
    });
    Ok(PreviewOutcome::Seen {
        preview: PreviewView {
            name: previewed.name,
            partial: previewed.partial,
            entries: previewed.entries.into_iter().map(EntryView::from).collect(),
            total: previewed.total,
            to_write: previewed.to_write,
            earlier: earlier.as_ref() == Some(&previewed.folder),
            // Received earlier elsewhere: its files there stay, and it starts
            // again in the new folder.
            earlier_elsewhere: earlier
                .filter(|earlier| *earlier != previewed.folder)
                .map(|earlier| earlier.display().to_string()),
            folder: previewed.folder.display().to_string(),
            left,
            saved_as: super::words::saved_as_views(&previewed.saved_as),
        },
    })
}

impl From<ourstick_core::download::TopEntry> for EntryView {
    fn from(entry: ourstick_core::download::TopEntry) -> Self {
        Self {
            shown: ourstick_core::untrusted::escape(
                entry.name.rsplit('/').next().unwrap_or(&entry.name),
            ),
            name: entry.name,
            is_dir: entry.is_dir,
            files: entry.files,
            bytes: entry.bytes,
            to_write: entry.to_write,
        }
    }
}

/// What is inside a top-level `folder` of the latest preview, when the
/// window opens it: each named by its whole path, with what it would write.
#[tauri::command]
pub fn preview_inside(
    folder: String,
    receivers: State<'_, super::Receivers>,
) -> Result<Vec<EntryView>, String> {
    let previewed = receivers
        .previewed()
        .ok_or_else(|| "Look at the sharing link again.".to_owned())?;
    Ok(ourstick_core::download::entries_in(
        &previewed.manifest,
        Some(&folder),
        Some(&previewed.to_write),
    )
    .into_iter()
    .map(EntryView::from)
    .collect())
}

/// What the latest preview would write with `left_out` not got: its whole
/// bytes to write, less each path left out (a set the core accepts).
#[tauri::command]
pub fn preview_needs(
    left_out: Vec<String>,
    receivers: State<'_, super::Receivers>,
) -> Result<u64, String> {
    let previewed = receivers
        .previewed()
        .ok_or_else(|| "Look at the sharing link again.".to_owned())?;
    let left_out = ourstick_core::download::LeftOut::normalised(&left_out)
        .map_err(|_| "That choice cannot be used. Choose again.".to_owned())?;
    Ok(needs(&previewed.to_write, &left_out.paths()))
}

/// Bytes to write by path (top-level keys and keys inside one), less the
/// paths left out: `left_out` normalised, so none lies under another.
fn needs(to_write: &std::collections::BTreeMap<String, u64>, left_out: &[String]) -> u64 {
    let all: u64 = to_write
        .iter()
        .filter(|(path, _)| !path.contains('/'))
        .map(|(_, bytes)| *bytes)
        .sum();
    let out: u64 = left_out.iter().filter_map(|path| to_write.get(path)).sum();
    all.saturating_sub(out)
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
