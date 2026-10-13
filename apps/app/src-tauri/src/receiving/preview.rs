//! A pasted sharing link's preview (step 7b): what the stick holds, where
//! it would be saved and whether it fits. Keeps nothing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ourstick_core::device::random_key;
use ourstick_core::download::preview;
use ourstick_core::received::check_link;
use ourstick_core::space::{available, SystemDisks};
use ourstick_core::stick_names::folder_name;
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
    Seen { preview: PreviewView },
    SenderAway,
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
        None => {
            let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
            let home = std::env::var_os(home).ok_or("OurStick cannot find your home folder.")?;
            Ok(PathBuf::from(home).join("Downloads"))
        }
    }
}

/// The folder a received stick goes into: one named after it in `parent`,
/// not taken already. Blocking (it looks at folders).
fn folder_for(name: &str, parent: &Path) -> PathBuf {
    parent.join(folder_name(name, |candidate| {
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
    let earlier = super::work(move || {
        check_link(store.as_ref(), &stick_id, StickKind::App)?;
        Ok(store
            .joined_stick(&stick_id)?
            .map(|joined| joined.output_dir))
    })
    .await?;
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
        "This computer's network could not be started. Check it, then try again.".to_owned()
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
        Ok(Err(error)) => return Err(crate::receiving::stopped_words(&error)),
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
mod tests {
    use super::*;

    #[test]
    fn what_a_choice_needs_leaves_out_each_path_once() {
        let to_write: std::collections::BTreeMap<String, u64> = [
            ("Disk".to_owned(), 10),
            ("Disk/MUSIC".to_owned(), 6),
            ("Disk/Pictures".to_owned(), 3),
            ("note".to_owned(), 1),
        ]
        .into();
        assert_eq!(needs(&to_write, &[]), 11);
        assert_eq!(needs(&to_write, &["Disk/MUSIC".to_owned()]), 5);
        assert_eq!(needs(&to_write, &["Disk".to_owned(), "note".to_owned()]), 0);
        // A path the plan writes nothing of (already there, or gone).
        assert_eq!(needs(&to_write, &["Disk/old".to_owned()]), 11);
    }

    #[test]
    fn a_stick_goes_into_a_folder_named_after_it_never_one_taken() {
        let parent = crate::testing::TempFolder::new();
        let folder = folder_for("Holiday", parent.path());
        assert_eq!(folder, parent.path().join("Holiday"));
        std::fs::create_dir(&folder).unwrap();
        let next = folder_for("Holiday", parent.path());
        assert_ne!(next, folder);
        assert!(next.starts_with(parent.path()));
        // A name with characters no folder may have still gets a folder.
        let odd = folder_for("a/b:c", parent.path());
        assert_eq!(odd.parent().unwrap(), parent.path());
    }

    #[test]
    fn a_preview_and_a_wait_choose_the_same_folder() {
        let parent = crate::testing::TempFolder::new();
        let chosen = parent.path().display().to_string();
        let earlier = parent.path().join("got with the command line");
        // A command-line receive goes on where it is, unless the user chose
        // another place.
        let choose = folder_choice(Some(earlier.clone()), None).unwrap();
        assert_eq!(choose("Holiday"), earlier);
        let choose = folder_choice(Some(earlier), Some(&chosen)).unwrap();
        assert_eq!(choose("Holiday"), parent.path().join("Holiday"));
        let choose = folder_choice(None, Some(&chosen)).unwrap();
        assert_eq!(choose("Holiday"), parent.path().join("Holiday"));
    }

    #[test]
    fn an_unreachable_sender_is_away_but_not_a_stick_it_no_longer_shares() {
        use ourstick_core::download::DownloadError;
        use ourstick_core::transport::TransportError;
        let unreachable = TransportError::Unreachable("gone".to_owned());
        assert!(sender_away(&DownloadError::Transport(unreachable)));
        assert!(!sender_away(&DownloadError::Transport(
            TransportError::UnknownStick
        )));
    }

    #[test]
    fn a_folder_not_made_yet_is_measured_where_it_will_be() {
        let parent = crate::testing::TempFolder::new();
        let deep = parent.path().join("a/b/c");
        assert_eq!(existing(&deep), parent.path());
    }
}
