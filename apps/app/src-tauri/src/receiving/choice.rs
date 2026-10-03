//! Choosing what to get of a received stick (ADR 0012): its top-level
//! entries with their ticks, and saving a choice.

use ourstick_core::device::open_system_keyring;
use ourstick_core::download::TopEntry;
use ourstick_core::store::Store;
use ourstick_core::untrusted::escape;
use ourstick_core::StickId;
use serde::Serialize;
use tauri::{AppHandle, State};

use super::{read_ticket, start, Receivers};
use crate::state::{stick_id as stick_id_of, AppState};

/// One top-level entry of a received stick, as the choice lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChoiceEntry {
    /// Exactly as the stick names it: what a choice is saved with.
    name: String,
    /// The same, escaped for the window (the sender's text).
    shown: String,
    is_dir: bool,
    files: u64,
    bytes: u64,
    ticked: bool,
    /// Files of it are already here (they stay if it is un-ticked).
    here: bool,
    /// On the stick now; an un-ticked name the stick no longer has is still
    /// listed, so a save never drops it.
    on_stick: bool,
}

/// What a received stick has at its top level, with what is ticked: the
/// latest round's entries (none yet if no round has read the manifest since
/// the app started), together with every name left out. Largest first.
pub fn choice_entries(
    store: &dyn Store,
    stick_id: &StickId,
    contents: Option<&[TopEntry]>,
) -> anyhow::Result<Vec<ChoiceEntry>> {
    let left_out = store.left_out(stick_id)?;
    let mut entries: Vec<ChoiceEntry> = Vec::new();
    for entry in contents.unwrap_or_default() {
        entries.push(ChoiceEntry {
            name: entry.name.clone(),
            shown: escape(&entry.name),
            is_dir: entry.is_dir,
            files: entry.files,
            bytes: entry.bytes,
            ticked: !left_out.contains(&entry.name),
            here: store.has_files_under(stick_id, &entry.name)?,
            on_stick: true,
        });
    }
    for name in &left_out {
        if !entries.iter().any(|entry| &entry.name == name) {
            entries.push(ChoiceEntry {
                name: name.clone(),
                shown: escape(name),
                is_dir: false,
                files: 0,
                bytes: 0,
                ticked: false,
                here: store.has_files_under(stick_id, name)?,
                on_stick: false,
            });
        }
    }
    entries.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

/// A top-level name left out, as the list says it: "Not getting: Films",
/// or "Not updating" for one whose files are already here.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct LeftOutView {
    shown: String,
    here: bool,
}

/// What a received stick leaves out, as its line in the list says it.
pub fn left_out_views(store: &dyn Store, stick_id: &StickId) -> anyhow::Result<Vec<LeftOutView>> {
    let mut views = Vec::new();
    for name in store.left_out(stick_id)? {
        views.push(LeftOutView {
            shown: escape(&name),
            here: store.has_files_under(stick_id, &name)?,
        });
    }
    Ok(views)
}

/// Saves a choice: the top-level names not to get (each a plain name; the
/// whole set replaces the last). The folder to receive the stick into again
/// if it is not paused (paused, the choice counts when it goes on).
pub fn save_choice(
    store: &dyn Store,
    stick_id: &StickId,
    left_out: &[String],
) -> anyhow::Result<Option<std::path::PathBuf>> {
    if left_out
        .iter()
        .any(|name| name.is_empty() || name.contains('/'))
    {
        anyhow::bail!("That choice cannot be saved. Choose again.");
    }
    store.set_left_out(stick_id, left_out)?;
    let joined = store
        .joined_stick(stick_id)?
        .ok_or_else(|| anyhow::anyhow!("This stick is not received here any more."))?;
    Ok((!joined.paused).then_some(joined.output_dir))
}

/// What the choice window lists for a received stick.
#[derive(Debug, Serialize)]
pub struct ChoiceView {
    entries: Vec<ChoiceEntry>,
    /// A round has read the stick's list since the app started: every entry
    /// is here, not only those already left out.
    known: bool,
}

/// "Choose what to get…": the received stick's top-level entries.
#[tauri::command]
pub async fn received_contents(
    stick_id: String,
    state: State<'_, AppState>,
    receivers: State<'_, Receivers>,
) -> Result<ChoiceView, String> {
    let stick_id = stick_id_of(&stick_id)?;
    let contents = receivers.contents(&stick_id);
    let known = contents.is_some();
    let store = state.store.clone();
    let entries =
        super::work(move || choice_entries(store.as_ref(), &stick_id, contents.as_deref())).await?;
    Ok(ChoiceView { entries, known })
}

/// Saves what the user chose not to get (the whole set), then receives the
/// stick again with it, unless it is paused (the choice counts when it goes
/// on).
#[tauri::command]
pub async fn choose_what_to_get(
    stick_id: String,
    left_out: Vec<String>,
    app: AppHandle,
    state: State<'_, AppState>,
    receivers: State<'_, Receivers>,
) -> Result<(), String> {
    let stick_id = stick_id_of(&stick_id)?;
    if !receivers.is_held(&stick_id).await {
        return Err("This stick is not received here any more.".to_owned());
    }
    let store = state.store.clone();
    let again = super::work(move || {
        let Some(folder) = save_choice(store.as_ref(), &stick_id, &left_out)? else {
            return Ok(None);
        };
        let secrets = open_system_keyring()?;
        Ok(Some((read_ticket(secrets.as_ref(), &stick_id)?, folder)))
    })
    .await?;
    if let Some((ticket, folder)) = again {
        start(&app, stick_id, ticket, folder, false).await;
    }
    super::changed(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ourstick_core::store::{DownloadedFile, JoinedStick, MemoryStore};

    fn top(name: &str, bytes: u64) -> TopEntry {
        TopEntry {
            name: name.to_owned(),
            is_dir: true,
            files: 1,
            bytes,
            to_write: bytes,
        }
    }

    fn received(store: &MemoryStore, seed: u8) -> StickId {
        let id = ourstick_core::StickSigningKey::from_bytes(&[seed; 32]).stick_id();
        store
            .add_joined_stick(&JoinedStick::new(id, "/d/x", 1))
            .unwrap();
        id
    }

    #[test]
    fn a_choice_is_saved_whole_and_the_stick_goes_on_unless_paused() {
        let store = MemoryStore::new();
        let id = received(&store, 5);
        let folder = save_choice(&store, &id, &["Films".to_owned()]).unwrap();
        assert_eq!(folder.as_deref(), Some(std::path::Path::new("/d/x")));
        assert_eq!(store.left_out(&id).unwrap(), ["Films"]);
        // Paused: saved, and nothing starts.
        store
            .change_joined_stick(&id, &ourstick_core::store::JoinedChange::Paused(true))
            .unwrap();
        assert_eq!(save_choice(&store, &id, &[]).unwrap(), None);
        assert!(store.left_out(&id).unwrap().is_empty());
        // Not plain names: refused, the last choice kept.
        store.set_left_out(&id, &["Music".to_owned()]).unwrap();
        for bad in ["", "a/b"] {
            assert!(save_choice(&store, &id, &[bad.to_owned()]).is_err());
        }
        assert_eq!(store.left_out(&id).unwrap(), ["Music"]);
        // A stick not received here: refused.
        let other = ourstick_core::StickSigningKey::from_bytes(&[6; 32]).stick_id();
        assert!(save_choice(&store, &other, &[]).is_err());
    }

    #[test]
    fn the_list_says_what_is_left_out_and_whether_it_is_here() {
        let store = MemoryStore::new();
        let id = received(&store, 7);
        assert!(left_out_views(&store, &id).unwrap().is_empty());
        store
            .set_left_out(&id, &["Films".to_owned(), "a\u{202e}".to_owned()])
            .unwrap();
        let file = DownloadedFile {
            path: "Films/x".to_owned(),
            saved_as: "Films/x".to_owned(),
            stamp: ourstick_core::manifest::FileStamp {
                size: 1,
                mtime_ns: 1,
                ctime_ns: 1,
            },
            blob_hash: [1; 32],
        };
        store.record_downloaded_file(&id, &file).unwrap();
        assert_eq!(
            left_out_views(&store, &id).unwrap(),
            [
                LeftOutView {
                    shown: "Films".to_owned(),
                    here: true
                },
                LeftOutView {
                    shown: "a\\u{202e}".to_owned(),
                    here: false
                },
            ]
        );
    }

    #[test]
    fn the_choice_lists_every_entry_and_every_name_left_out() {
        let store = MemoryStore::new();
        let id = ourstick_core::StickSigningKey::from_bytes(&[4; 32]).stick_id();
        store
            .add_joined_stick(&JoinedStick::new(id, "/d/x", 1))
            .unwrap();
        store
            .set_left_out(&id, &["Films".to_owned(), "Gone".to_owned()])
            .unwrap();
        let file = DownloadedFile {
            path: "Films/a.mkv".to_owned(),
            saved_as: "Films/a.mkv".to_owned(),
            stamp: ourstick_core::manifest::FileStamp {
                size: 1,
                mtime_ns: 1,
                ctime_ns: 1,
            },
            blob_hash: [1; 32],
        };
        store.record_downloaded_file(&id, &file).unwrap();
        let contents = [top("Music", 5), top("Films", 9), top("a\u{1b}b", 1)];
        let entries = choice_entries(&store, &id, Some(&contents)).unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        // Largest first; a name left out that the stick no longer has, last.
        assert_eq!(names, ["Films", "Music", "a\u{1b}b", "Gone"]);
        assert!(!entries[0].ticked && entries[0].here && entries[0].on_stick);
        assert!(entries[1].ticked && !entries[1].here);
        assert_eq!(entries[2].shown, "a\\u{1b}b");
        assert!(!entries[3].ticked && !entries[3].on_stick);
        // Before any round: only what is left out.
        let before = choice_entries(&store, &id, None).unwrap();
        assert_eq!(before.len(), 2);
    }
}
