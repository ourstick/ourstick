//! Linux: `~/OurStick`, bookmarked in the file manager's sidebar (GTK and
//! KDE). Eject removes the bookmark.

use std::path::Path;

use super::NAME;

/// `path` as a `file://` URI, percent-encoded.
fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

// ---------------------------------------------------------------- Linux

/// GTK bookmarks (Nautilus and other GTK file managers): one `URI name` per
/// line. Adds ours unless it is there.
fn with_gtk_bookmark(content: &str, uri: &str) -> String {
    if content
        .lines()
        .any(|line| line.split(' ').next() == Some(uri))
    {
        return content.to_owned();
    }
    let mut content = content.to_owned();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&format!("{uri} {NAME}\n"));
    content
}

fn without_gtk_bookmark(content: &str, uri: &str) -> String {
    content
        .lines()
        .filter(|line| line.split(' ').next() != Some(uri))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// KDE's places (Dolphin): an XBEL file. Ours gets the USB drive icon.
fn xbel_bookmark(uri: &str) -> String {
    format!(
        "  <bookmark href=\"{uri}\">\n   <title>{NAME}</title>\n   <info>\n    \
         <metadata owner=\"http://freedesktop.org\">\n     \
         <bookmark:icon name=\"drive-removable-media-usb\"/>\n    </metadata>\n   \
         </info>\n  </bookmark>\n"
    )
}

fn with_xbel_bookmark(content: &str, uri: &str) -> Option<String> {
    if content.contains(&format!("<bookmark href=\"{uri}\"")) {
        return Some(content.to_owned());
    }
    let end = content.rfind("</xbel>")?;
    Some(format!(
        "{}{}{}",
        &content[..end],
        xbel_bookmark(uri),
        &content[end..]
    ))
}

fn without_xbel_bookmark(content: &str, uri: &str) -> String {
    let start_tag = format!("<bookmark href=\"{uri}\"");
    let Some(start) = content.find(&start_tag) else {
        return content.to_owned();
    };
    let Some(length) = content[start..].find("</bookmark>") else {
        return content.to_owned();
    };
    let end = start + length + "</bookmark>".len();
    // Take the whole lines, so no blank line is left behind.
    let start = content[..start].rfind('\n').map_or(0, |at| at + 1);
    let end = content[end..]
        .find('\n')
        .map_or(content.len(), |at| end + at + 1);
    format!("{}{}", &content[..start], &content[end..])
}

#[cfg(target_os = "linux")]
pub mod platform {
    use super::*;
    use crate::drive::{folder_is_there, home};
    use anyhow::{bail, Context, Result};
    use ourstick_core::manifest::ignore::is_ignored;
    use std::path::PathBuf;

    fn gtk_bookmarks() -> Result<PathBuf> {
        let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => home()?.join(".config"),
        };
        Ok(config.join("gtk-3.0/bookmarks"))
    }

    /// Dolphin's places, only if the user has KDE's file (never made here).
    fn kde_places() -> Result<PathBuf> {
        Ok(home()?.join(".local/share/user-places.xbel"))
    }

    fn edit(path: &Path, change: impl Fn(&str) -> Option<String>) -> Result<()> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()))
            }
        };
        if let Some(changed) = change(&content) {
            if changed != content {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, changed)
                    .with_context(|| format!("cannot write {}", path.display()))?;
            }
        }
        Ok(())
    }

    /// Makes `~/OurStick` for a new stick. An existing one is used only if it
    /// is empty (system files aside) or already a stick's; anything else is
    /// the user's and is left alone.
    pub fn create(_size: u64) -> Result<PathBuf> {
        let root = home()?.join(NAME);
        if root.exists() {
            let has_own_files = std::fs::read_dir(&root)
                .with_context(|| format!("cannot open {}", root.display()))?
                .any(|entry| {
                    entry.is_ok_and(|entry| !entry.file_name().to_str().is_some_and(is_ignored))
                });
            if has_own_files && !folder_is_there(&root) {
                bail!(
                    "{} already exists and holds files. OurStick will not take it over; \
                     move it away and try again.",
                    root.display()
                );
            }
        } else {
            std::fs::create_dir(&root)
                .with_context(|| format!("cannot create {}", root.display()))?;
        }
        // A USB drive icon for the folder where the file manager honours it.
        let _ = std::process::Command::new("gio")
            .arg("set")
            .arg(&root)
            .arg("metadata::custom-icon-name")
            .arg("drive-removable-media-usb")
            .status();
        mount(&root)
    }

    /// Puts the stick in the sidebar; the folder itself is always there.
    pub fn mount(root: &Path) -> Result<PathBuf> {
        let uri = file_uri(root);
        edit(&gtk_bookmarks()?, |content| {
            Some(with_gtk_bookmark(content, &uri))
        })?;
        let places = kde_places()?;
        if places.exists() {
            edit(&places, |content| with_xbel_bookmark(content, &uri))?;
        }
        Ok(root.to_path_buf())
    }

    /// Takes the stick out of the sidebar. The folder stays, and so does
    /// sharing.
    pub fn eject(root: &Path) -> Result<()> {
        let uri = file_uri(root);
        edit(&gtk_bookmarks()?, |content| {
            Some(without_gtk_bookmark(content, &uri))
        })?;
        let places = kde_places()?;
        if places.exists() {
            edit(&places, |content| {
                Some(without_xbel_bookmark(content, &uri))
            })?;
        }
        Ok(())
    }

    pub fn is_mounted(root: &Path) -> bool {
        let uri = file_uri(root);
        gtk_bookmarks()
            .and_then(|path| Ok(std::fs::read_to_string(path)?))
            .is_ok_and(|content| {
                content
                    .lines()
                    .any(|line| line.split(' ').next() == Some(uri.as_str()))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URI: &str = "file:///home/anna/OurStick";

    #[test]
    fn a_path_becomes_a_file_uri() {
        assert_eq!(file_uri(Path::new("/home/anna/OurStick")), URI);
        assert_eq!(
            file_uri(Path::new("/home/a b/Ö")),
            "file:///home/a%20b/%C3%96"
        );
    }

    #[test]
    fn the_gtk_bookmark_is_added_once_and_removed_alone() {
        let theirs = "file:///home/anna/Music\nfile:///home/anna/Work Work\n";
        let with = with_gtk_bookmark(theirs, URI);
        assert_eq!(with, format!("{theirs}{URI} OurStick\n"));
        assert_eq!(with_gtk_bookmark(&with, URI), with);
        assert_eq!(without_gtk_bookmark(&with, URI), theirs);
        assert_eq!(with_gtk_bookmark("", URI), format!("{URI} OurStick\n"));
        // A missing final newline in their file does not glue the lines.
        assert_eq!(
            with_gtk_bookmark("file:///x", URI),
            format!("file:///x\n{URI} OurStick\n")
        );
    }

    #[test]
    fn the_kde_place_is_added_once_and_removed_alone() {
        let theirs = "<?xml version=\"1.0\"?>\n<xbel>\n  <bookmark href=\"file:///home/anna\">\n   <title>Home</title>\n  </bookmark>\n</xbel>\n";
        let with = with_xbel_bookmark(theirs, URI).unwrap();
        assert!(with.contains("drive-removable-media-usb"));
        assert!(with.ends_with("</bookmark>\n</xbel>\n"));
        assert_eq!(with_xbel_bookmark(&with, URI).unwrap(), with);
        assert_eq!(without_xbel_bookmark(&with, URI), theirs);
        assert_eq!(without_xbel_bookmark(theirs, URI), theirs);
        assert_eq!(with_xbel_bookmark("not xbel", URI), None);
    }
}
