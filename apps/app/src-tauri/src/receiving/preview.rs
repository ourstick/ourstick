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

/// How long a preview waits for the sender.
const PREVIEW_WAIT: Duration = Duration::from_secs(30);
/// How many top-level entries a preview lists; the rest are counted.
/// Every top-level entry is listed: each must be choosable (ADR 0012).
const LISTED: usize = usize::MAX;

/// A top-level entry of a previewed stick.
#[derive(Debug, Serialize)]
pub struct EntryView {
    /// Exactly as the stick names it: what a choice is made with (shown
    /// isolated; the window escapes nothing, Svelte writes text only).
    name: String,
    is_dir: bool,
    files: u64,
    bytes: u64,
    /// What it would write into the folder (ADR 0012): the window sums the
    /// ticked ones.
    to_write: u64,
}

/// What adding a pasted sharing link would bring.
#[derive(Debug, Serialize)]
pub struct PreviewView {
    name: String,
    /// The sender is still counting its files: more is to come.
    partial: bool,
    entries: Vec<EntryView>,
    /// Entries past the first [`LISTED`].
    more: usize,
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
) -> Result<PreviewView, String> {
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
    let fetcher = IrohFetcher::start(&config, &blobs).await.map_err(|error| {
        tracing::warn!(%error, "a preview's networking");
        "This computer's network could not be started. Check it, then try again.".to_owned()
    })?;
    let chosen_earlier = earlier.clone();
    let chosen = parent.is_some();
    let parent = parent_folder(parent.as_deref())?;
    let choose = move |name: &str| match chosen_earlier {
        Some(folder) if !chosen => folder,
        _ => folder_for(name, &parent),
    };
    let looked = tokio::time::timeout(
        PREVIEW_WAIT,
        preview(&fetcher, state.store.clone(), &ticket, choose),
    )
    .await;
    let _ = fetcher.shutdown(false).await;
    let _ = tauri::async_runtime::spawn_blocking(move || std::fs::remove_dir_all(&blobs)).await;
    let previewed =
        match looked {
            Err(_) => return Err(
                "The sender's computer is off, or OurStick is not open there. Try again when it \
                 is."
                .to_owned(),
            ),
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
    let more = previewed.entries.len().saturating_sub(LISTED);
    Ok(PreviewView {
        name: previewed.name,
        partial: previewed.partial,
        entries: previewed
            .entries
            .into_iter()
            .take(LISTED)
            .map(|entry| EntryView {
                name: entry.name,
                is_dir: entry.is_dir,
                files: entry.files,
                bytes: entry.bytes,
                to_write: entry.to_write,
            })
            .collect(),
        more,
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_folder_not_made_yet_is_measured_where_it_will_be() {
        let parent = crate::testing::TempFolder::new();
        let deep = parent.path().join("a/b/c");
        assert_eq!(existing(&deep), parent.path());
    }
}
