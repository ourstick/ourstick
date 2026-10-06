//! The lines that say what is saved under another name (ADR 0010, rule 8):
//! a renamed folder once for its files, else the file. One grouping for the
//! round's events, the preview and the receiver's notice.

use std::collections::HashMap;

use super::saved_paths::SavedAs;

/// What `files` (recorded rows of one folder) hold under another name than
/// theirs, a renamed folder once for its files, by path: the receiver's
/// notice, from what is recorded (ADR 0010, rule 8).
pub fn saved_elsewhere(files: &[crate::store::DownloadedFile]) -> Vec<SavedAs> {
    let mut files: Vec<&crate::store::DownloadedFile> = files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    let saved: Vec<String> = files.iter().map(|file| file.saved_as.clone()).collect();
    renamed(&paths, &saved)
}

/// What was saved under another name, a renamed folder once for its files.
pub(super) fn renamed(paths: &[&str], saved: &[impl AsRef<str>]) -> Vec<SavedAs> {
    let mut lines: Vec<SavedAs> = Vec::new();
    let mut index_of: HashMap<(String, String), usize> = HashMap::new();
    for (path, saved_as) in paths.iter().zip(saved) {
        let saved_as = saved_as.as_ref();
        if *path == saved_as {
            continue;
        }
        // The first name that differs: a folder for all its files, or the
        // file itself.
        let stick: Vec<&str> = path.split('/').collect();
        let here: Vec<&str> = saved_as.split('/').collect();
        if stick.len() != here.len() {
            // A row not of this path's shape (saved_paths chooses it again).
            continue;
        }
        let differs = stick
            .iter()
            .zip(&here)
            .position(|(a, b)| a != b)
            .unwrap_or(stick.len() - 1);
        let key = (stick[..=differs].join("/"), here[..=differs].join("/"));
        match index_of.get(&key) {
            Some(&index) => lines[index].files += 1,
            None => {
                index_of.insert(key.clone(), lines.len());
                lines.push(SavedAs {
                    path: key.0,
                    saved_as: key.1,
                    files: 1,
                });
            }
        }
    }
    lines
}
