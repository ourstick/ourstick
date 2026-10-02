//! A pasted sharing link's preview (step 7b): what the stick holds, where
//! it would be saved and whether it fits. Keeps nothing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ourstick_core::device::random_key;
use ourstick_core::download::{preview, Preview};
use ourstick_core::received::check_link;
use ourstick_core::space::{available, SystemDisks};
use ourstick_core::stick_names::folder_name;
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, Network};
use ourstick_core::transport::Fetcher;
use ourstick_core::StickKind;
use serde::Serialize;
use tauri::State;

use super::commands::ticket_of;
use crate::commands::blocking;
use crate::state::AppState;

/// How long a preview waits for the sender.
const PREVIEW_WAIT: Duration = Duration::from_secs(30);
/// How many top-level entries a preview lists; the rest are counted.
const LISTED: usize = 200;

/// A top-level entry of a previewed stick.
#[derive(Debug, Serialize)]
pub struct EntryView {
    name: String,
    is_dir: bool,
    files: u64,
    bytes: u64,
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
    /// What is left there for it.
    left: u64,
}

/// The folder a received stick goes into: `parent` (else the Downloads
/// folder), and a folder named after the stick in it, not taken already.
fn folder_for(name: &str, parent: Option<&str>) -> Result<PathBuf, String> {
    let parent = match parent {
        Some(parent) => PathBuf::from(parent),
        None => {
            let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
            let home = std::env::var_os(home).ok_or("cannot find your home folder")?;
            PathBuf::from(home).join("Downloads")
        }
    };
    let folder = folder_name(name, |candidate| parent.join(candidate).exists());
    Ok(parent.join(folder))
}

/// The nearest folder of `path` that exists, to measure its disk.
fn existing(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|ancestor| ancestor.is_dir())
        .unwrap_or(path)
        .to_path_buf()
}

/// "Add a sharing link…": what the stick holds, where it would go and
/// whether it fits. Waits for the sender at most half a minute.
#[tauri::command]
pub async fn preview_link(
    text: String,
    parent: Option<String>,
    state: State<'_, AppState>,
) -> Result<PreviewView, String> {
    let ticket = ticket_of(&text)?;
    let store = state.store.clone();
    let stick_id = ticket.stick_id;
    blocking(move || Ok(check_link(store.as_ref(), &stick_id, StickKind::App)?)).await?;
    // A throwaway blob store: a preview keeps nothing.
    let hex: String = random_key()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let blobs = state.data_dir.join("previews").join(hex);
    let config = IrohConfig::new(random_key(), Network::Public);
    let fetcher = IrohFetcher::start(&config, &blobs)
        .await
        .map_err(|error| format!("cannot start networking: {error}"))?;
    // The folder is decided once the name is known: preview into the
    // default first, then measured where it really goes.
    let named = tokio::time::timeout(
        PREVIEW_WAIT,
        preview(&fetcher, state.store.clone(), &ticket, Path::new("")),
    )
    .await;
    let previewed: Result<Preview, String> = match named {
        Err(_) => Err(
            "The sender's computer is off, or OurStick is not open there. Try again when it is."
                .to_owned(),
        ),
        Ok(Err(error)) => Err(crate::receiving::stopped_words(&error)),
        Ok(Ok(previewed)) => Ok(previewed),
    };
    let previewed = match previewed {
        Ok(previewed) => {
            let folder = folder_for(&previewed.name, parent.as_deref())?;
            match tokio::time::timeout(
                PREVIEW_WAIT,
                preview(&fetcher, state.store.clone(), &ticket, &folder),
            )
            .await
            {
                Ok(Ok(here)) => Ok((here, folder)),
                Ok(Err(error)) => Err(crate::receiving::stopped_words(&error)),
                Err(_) => Ok((previewed, folder)),
            }
        }
        Err(error) => Err(error),
    };
    let _ = fetcher.shutdown(false).await;
    let _ = std::fs::remove_dir_all(&blobs);
    let (previewed, folder) = previewed?;
    let store = state.store.clone();
    let data_dir = state.data_dir.clone();
    let measured = existing(&folder);
    let left = blocking(move || {
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
            })
            .collect(),
        more,
        total: previewed.total,
        to_write: previewed.to_write,
        folder: folder.display().to_string(),
        left,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stick_goes_into_a_folder_named_after_it_never_one_taken() {
        let parent = crate::testing::TempFolder::new();
        let parent_text = parent.path().to_str().unwrap();
        let folder = folder_for("Holiday", Some(parent_text)).unwrap();
        assert_eq!(folder, parent.path().join("Holiday"));
        std::fs::create_dir(&folder).unwrap();
        let next = folder_for("Holiday", Some(parent_text)).unwrap();
        assert_ne!(next, folder);
        assert!(next.starts_with(parent.path()));
        // A name with characters no folder may have still gets a folder.
        let odd = folder_for("a/b:c", Some(parent_text)).unwrap();
        assert_eq!(odd.parent().unwrap(), parent.path());
    }

    #[test]
    fn a_folder_not_made_yet_is_measured_where_it_will_be() {
        let parent = crate::testing::TempFolder::new();
        let deep = parent.path().join("a/b/c");
        assert_eq!(existing(&deep), parent.path());
    }
}
