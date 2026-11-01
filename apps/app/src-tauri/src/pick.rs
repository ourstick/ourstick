//! What is chosen to be sent: the system's pickers (ADR 0028 §2), and on a
//! phone whether what was chosen stays readable (ADR 0029).

use std::collections::HashSet;
use std::path::Path;

use tauri::AppHandle;

/// What to choose with the system's picker: files, or folders
/// (Linux's and Windows' pickers do not take both at once); on a phone
/// also photos and videos, with its photo picker (ADR 0029).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pick {
    Files,
    Folders,
    Photos,
}

/// `Choose folders…` / `Choose files…`: the system's picker, several at
/// once; what is chosen is then checked as a drop is. Empty if the user
/// cancelled.
#[cfg(desktop)]
#[tauri::command]
pub async fn choose_paths(pick: Pick, app: AppHandle) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file().set_title("Choose what to send");
        match pick {
            Pick::Files | Pick::Photos => dialog.blocking_pick_files(),
            Pick::Folders => dialog.blocking_pick_folders(),
        }
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(|picked| picked.into_path().ok())
        .map(|path| path.display().to_string())
        .collect())
}

/// Android's pickers (ADR 0029): each chosen place as the path the core
/// reads it by, its content URI in it.
#[cfg(target_os = "android")]
#[tauri::command]
pub async fn choose_paths(pick: Pick, app: AppHandle) -> Result<Vec<String>, String> {
    let chosen = tauri::async_runtime::spawn_blocking(move || crate::phone::choose(&app, pick))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))?;
    Ok(chosen
        .into_iter()
        .map(|(uri, name)| {
            crate::documents::paths::place_path(&uri, &name)
                .display()
                .to_string()
        })
        .collect())
}

/// Sending from an iPhone is not there yet (ADR 0029).
#[cfg(target_os = "ios")]
#[tauri::command]
pub async fn choose_paths(_pick: Pick, _app: AppHandle) -> Result<Vec<String>, String> {
    Err("An iPhone cannot send yet.".to_owned())
}

/// "Find it…": the system's picker for one folder (`folder`) or one file,
/// for the entry `name`; `None` if the user cancelled.
#[cfg(desktop)]
pub async fn pick_one(app: &AppHandle, name: &str, folder: bool) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app
        .dialog()
        .file()
        .set_title(format!("Where is \u{201c}{name}\u{201d} now?"));
    let picked = tauri::async_runtime::spawn_blocking(move || {
        if folder {
            dialog.blocking_pick_folder()
        } else {
            dialog.blocking_pick_file()
        }
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(picked
        .and_then(|picked| picked.into_path().ok())
        .map(|path| path.display().to_string()))
}

/// Android's pickers (ADR 0029): the first place chosen, as its path.
#[cfg(target_os = "android")]
pub async fn pick_one(
    app: &AppHandle,
    _name: &str,
    folder: bool,
) -> Result<Option<String>, String> {
    let pick = if folder { Pick::Folders } else { Pick::Files };
    let app = app.clone();
    let chosen = tauri::async_runtime::spawn_blocking(move || crate::phone::choose(&app, pick))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{error:#}"))?;
    Ok(chosen.into_iter().next().map(|(uri, name)| {
        crate::documents::paths::place_path(&uri, &name)
            .display()
            .to_string()
    }))
}

/// Sending from an iPhone is not there yet (ADR 0029).
#[cfg(target_os = "ios")]
pub async fn pick_one(
    _app: &AppHandle,
    _name: &str,
    _folder: bool,
) -> Result<Option<String>, String> {
    Err("An iPhone cannot send yet.".to_owned())
}

/// The read grants a phone kept for what it chose (ADR 0029); `None`
/// elsewhere, or when the phone does not say. Blocking.
pub fn kept_here(app: &AppHandle) -> Option<HashSet<String>> {
    #[cfg(target_os = "android")]
    return crate::phone::kept_grants(app)
        .inspect_err(|error| tracing::warn!(%error, "the kept read grants are not known"))
        .ok();
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        None
    }
}

/// Why `path`, chosen on a phone, cannot be sent: the system let OurStick
/// read it only while it stays open, so the link would stop working
/// after OurStick closes.
#[cfg(any(target_os = "android", test))]
pub fn not_kept(kept: Option<&HashSet<String>>, path: &Path, name: &str) -> Option<String> {
    let kept = kept?;
    let chosen = crate::documents::paths::chosen(path)?;
    (!kept.contains(&chosen.uri)).then(|| {
        format!(
            "This phone lets OurStick read “{name}” only while OurStick is open, \
             so it cannot be sent. Choose the folder it is in instead."
        )
    })
}

#[cfg(not(any(target_os = "android", test)))]
pub fn not_kept(kept: Option<&HashSet<String>>, path: &Path, name: &str) -> Option<String> {
    let _ = (kept, path, name);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::paths::place_path;

    #[test]
    fn only_a_place_whose_grant_is_not_kept_is_refused() {
        let uri = "content://com.android.providers.media.documents/document/image%3A12";
        let place = place_path(uri, "IMG_1.jpg");
        let kept: HashSet<String> = [uri.to_owned()].into();
        assert_eq!(not_kept(Some(&kept), &place, "IMG_1.jpg"), None);
        assert_eq!(
            not_kept(Some(&kept), &place.join("inside.jpg"), "inside.jpg"),
            None
        );
        let other = place_path("content://example/document/2", "Notes.pdf");
        let why = not_kept(Some(&kept), &other, "Notes.pdf").unwrap();
        assert!(why.contains("“Notes.pdf”"), "{why}");
        // Not known, or not a phone's place: nothing to refuse here.
        assert_eq!(not_kept(None, &other, "Notes.pdf"), None);
        assert_eq!(
            not_kept(Some(&kept), Path::new("/home/alice/Notes.pdf"), "Notes.pdf"),
            None
        );
    }
}
